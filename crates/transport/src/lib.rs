//! Local IPC and private configuration adapters, shared by both binaries.
use runtime_protocol::*;
use std::{io, path::Path, time::Duration};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("IPC I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid IPC encoding: {0}")]
    Encoding(String),
    #[error("IPC frame exceeds limit")]
    FrameLimit,
    #[error("IPC deadline expired")]
    Deadline,
    #[error("IPC peer rejected request: {0:?}")]
    Peer(ErrorEnvelope),
    #[error("unexpected IPC frame")]
    Unexpected,
}
pub type Result<T> = std::result::Result<T, IpcError>;

pub async fn send<W: AsyncWrite + Unpin>(stream: &mut W, frame: &Frame) -> Result<()> {
    let data = rmp_serde::to_vec_named(frame).map_err(|e| IpcError::Encoding(e.to_string()))?;
    if data.len() > MAX_FRAME {
        return Err(IpcError::FrameLimit);
    }
    stream.write_u32(data.len() as u32).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}
pub async fn receive<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Frame> {
    let len = stream.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(IpcError::FrameLimit);
    }
    let mut data = vec![0; len];
    stream.read_exact(&mut data).await?;
    rmp_serde::from_slice(&data).map_err(|e| IpcError::Encoding(e.to_string()))
}
pub async fn authenticate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    secret: &[u8],
    workspace_id: uuid::Uuid,
) -> Result<()> {
    let hello = tokio::time::timeout(Duration::from_secs(5), receive(stream))
        .await
        .map_err(|_| IpcError::Deadline)??;
    let failure = match hello {
        Frame::Hello {
            protocol_version,
            secret: supplied,
        } => {
            if protocol_version != VERSION {
                Some(ErrorEnvelope::new(
                    ErrorCode::VersionMismatch,
                    "unsupported IPC version",
                ))
            } else if secret.len() < 32 || !bool::from(secret.ct_eq(&supplied)) {
                Some(ErrorEnvelope::new(
                    ErrorCode::Unauthorized,
                    "IPC authentication failed",
                ))
            } else {
                None
            }
        }
        _ => Some(ErrorEnvelope::new(
            ErrorCode::Unauthorized,
            "handshake required",
        )),
    };
    if let Some(err) = failure {
        send(stream, &Frame::Error(err.clone())).await?;
        return Err(IpcError::Peer(err));
    }
    send(
        stream,
        &Frame::Welcome {
            protocol_version: VERSION,
            workspace_id,
        },
    )
    .await
}

pub use runtime_ports::IpcTransportPort as Stream;
pub type LocalStream = Box<dyn Stream>;

#[cfg(windows)]
pub struct Listener {
    endpoint: String,
    next: tokio::net::windows::named_pipe::NamedPipeServer,
}
#[cfg(windows)]
impl Listener {
    pub async fn bind(endpoint: &str) -> Result<Self> {
        if !endpoint.starts_with(r"\\.\pipe\") {
            return Err(IpcError::Unexpected);
        }
        Ok(Self {
            endpoint: endpoint.into(),
            next: windows::create_pipe(endpoint, true)?,
        })
    }
    pub async fn accept(&mut self) -> Result<LocalStream> {
        self.next.connect().await?;
        let new = windows::create_pipe(&self.endpoint, false)?;
        Ok(Box::new(std::mem::replace(&mut self.next, new)))
    }
}
#[cfg(windows)]
async fn connect(endpoint: &str) -> Result<LocalStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    for _ in 0..50 {
        match ClientOptions::new().open(endpoint) {
            Ok(s) => return Ok(Box::new(s)),
            Err(e) if e.raw_os_error() == Some(231) => {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(IpcError::Deadline)
}
#[cfg(unix)]
pub struct Listener {
    inner: tokio::net::UnixListener,
    endpoint: std::path::PathBuf,
}
#[cfg(unix)]
impl Listener {
    pub async fn bind(endpoint: &str) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let path = Path::new(endpoint);
        let parent = path.parent().ok_or(IpcError::Unexpected)?;
        private_directory(parent)?;
        // Only remove a stale socket after confirming no live daemon owns it.
        if path.exists() {
            if tokio::net::UnixStream::connect(path).await.is_ok() {
                return Err(
                    io::Error::new(io::ErrorKind::AddrInUse, "daemon already listening").into(),
                );
            }
            use std::os::unix::fs::FileTypeExt;
            if !std::fs::symlink_metadata(path)?.file_type().is_socket() {
                return Err(IpcError::Unexpected);
            }
            std::fs::remove_file(path)?;
        }
        let inner = tokio::net::UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            inner,
            endpoint: path.into(),
        })
    }
    pub async fn accept(&mut self) -> Result<LocalStream> {
        Ok(Box::new(self.inner.accept().await?.0))
    }
}
#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.endpoint);
    }
}
#[cfg(unix)]
async fn connect(endpoint: &str) -> Result<LocalStream> {
    Ok(Box::new(tokio::net::UnixStream::connect(endpoint).await?))
}

#[derive(Clone)]
pub struct Client {
    endpoint: String,
    secret: Vec<u8>,
}
impl Client {
    pub fn new(endpoint: String, secret: Vec<u8>) -> Self {
        Self { endpoint, secret }
    }
    pub async fn request(
        &self,
        request: &RequestEnvelope,
        mut progress: impl FnMut(EventEnvelope),
    ) -> Result<ResponseEnvelope> {
        let remaining = request.meta.deadline.saturating_sub(now_ms());
        tokio::time::timeout(Duration::from_millis(remaining), async {
            for attempt in 0..4 {
                match self.once(request, &mut progress).await {
                    Ok(r) => return Ok(r),
                    Err(IpcError::Io(_)) if attempt < 3 => {
                        tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(IpcError::Deadline)
        })
        .await
        .map_err(|_| IpcError::Deadline)?
    }
    async fn once(
        &self,
        request: &RequestEnvelope,
        progress: &mut impl FnMut(EventEnvelope),
    ) -> Result<ResponseEnvelope> {
        let mut stream = connect(&self.endpoint).await?;
        send(
            &mut stream,
            &Frame::Hello {
                protocol_version: VERSION,
                secret: self.secret.clone(),
            },
        )
        .await?;
        match receive(&mut stream).await? {
            Frame::Welcome {
                protocol_version: VERSION,
                ..
            } => {}
            Frame::Error(e) => return Err(IpcError::Peer(e)),
            _ => return Err(IpcError::Unexpected),
        }
        send(&mut stream, &Frame::Request(request.clone())).await?;
        loop {
            match receive(&mut stream).await? {
                Frame::Response(r) if r.meta.request_id == request.meta.request_id => return Ok(r),
                Frame::Event(e) if e.meta.request_id == request.meta.request_id => progress(e),
                Frame::Error(e) => return Err(IpcError::Peer(e)),
                _ => return Err(IpcError::Unexpected),
            }
        }
    }
}

/// Private local configuration, never workspace file operations.
pub fn private_directory(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state directory cannot be a link",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    windows::private_acl(path)?;
    Ok(())
}
pub fn read_secret(path: &Path) -> io::Result<Vec<u8>> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "secret cannot be a link",
        ));
    }
    let secret = std::fs::read(path)?;
    if secret.len() != 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secret must contain 32 bytes",
        ));
    }
    Ok(secret)
}
/// Protect an existing operator-owned local file (backups and audit logs).
pub fn private_file(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private file must be regular",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    windows::private_acl(path)?;
    Ok(())
}
#[cfg(windows)]
mod windows;
