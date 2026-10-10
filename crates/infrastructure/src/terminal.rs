use crate::{
    files::WorkspaceFiles,
    processes::{
        ProgramRule,
        process_spool::{SpoolBudget, hex},
    },
    storage::{LocalStore, storage_error},
};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use runtime_domain::*;
use runtime_ports::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{RwLock, oneshot};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    pub enabled: bool,
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
    pub fingerprint: Option<String>,
    pub max_terminals: usize,
    pub max_input_bytes: usize,
    pub idle_timeout_seconds: u64,
}
impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            program: None,
            args: Vec::new(),
            fingerprint: None,
            max_terminals: 4,
            max_input_bytes: 4096,
            idle_timeout_seconds: 900,
        }
    }
}
impl TerminalConfig {
    pub fn validate(&self) -> Result<()> {
        if self.max_terminals == 0
            || self.max_terminals > 16
            || self.max_input_bytes == 0
            || self.max_input_bytes > 65536
            || self.idle_timeout_seconds == 0
            || self.idle_timeout_seconds > 86400
        {
            return Err(PortError::Policy("invalid terminal limits".into()));
        }
        if self.enabled
            && (self.program.as_ref().is_none_or(|p| !p.is_absolute())
                || self
                    .fingerprint
                    .as_ref()
                    .is_none_or(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit())))
        {
            return Err(PortError::Policy(
                "terminal requires local absolute program and SHA256 approval".into(),
            ));
        }
        Ok(())
    }
}
type Input = (Vec<u8>, oneshot::Sender<Result<()>>);
struct Live {
    workspace: WorkspaceId,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    writer: std::sync::mpsc::SyncSender<Input>,
    last_used: std::sync::atomic::AtomicU64,
    #[cfg(unix)]
    group: i32,
}
impl Live {
    fn terminate(&self) -> Result<()> {
        #[cfg(unix)]
        crate::terminal_guardian::kill_tree(self.group);
        self.killer
            .lock()
            .map_err(|_| PortError::Tool("terminal poisoned".into()))?
            .kill()
            .map_err(crate::files::io_error)
    }
}
impl Drop for TerminalManager {
    fn drop(&mut self) {
        if let Ok(states) = self.live.try_read() {
            for live in states.values() {
                let _ = live.terminate();
            }
        }
    }
}
pub struct TerminalManager {
    config: TerminalConfig,
    guardian: Option<PathBuf>,
    files: WorkspaceFiles,
    store: Arc<LocalStore>,
    spool: SpoolBudget,
    live: Arc<RwLock<HashMap<TerminalId, Arc<Live>>>>,
}
fn pty_error(error: impl std::fmt::Display) -> PortError {
    PortError::Tool(format!("terminal: {error}"))
}
fn terminal_id(input: &Value) -> Result<TerminalId> {
    input["terminal_id"]
        .as_str()
        .ok_or(PortError::NotFound)?
        .parse()
        .map_err(pty_error)
}
fn size(input: &Value) -> Result<PtySize> {
    let rows = input["rows"].as_u64().unwrap_or(24);
    let cols = input["cols"].as_u64().unwrap_or(80);
    if !(1..=1000).contains(&rows) || !(1..=1000).contains(&cols) {
        return Err(PortError::Policy(
            "terminal geometry outside 1..1000".into(),
        ));
    }
    Ok(PtySize {
        rows: rows as u16,
        cols: cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    })
}
impl TerminalManager {
    pub(crate) fn new(
        files: WorkspaceFiles,
        store: Arc<LocalStore>,
        spool: SpoolBudget,
        config: TerminalConfig,
        guardian: Option<PathBuf>,
    ) -> Self {
        Self {
            files,
            store,
            spool,
            config,
            guardian,
            live: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub async fn recover(&self, workspace: WorkspaceId) -> Result<()> {
        self.store.run(move|c|async move{c.execute("UPDATE terminals SET status='interrupted' WHERE workspace_id=?1 AND status='Running'",[workspace.to_string()]).await.map_err(storage_error)?;Ok(())}).await
    }
    async fn owned(&self, workspace: WorkspaceId, id: TerminalId) -> Result<Value> {
        self.store.run(move|c|async move{
            let mut rows=c.query("SELECT operation_id,status,exit_code FROM terminals WHERE id=?1 AND workspace_id=?2",libsql::params![id.to_string(),workspace.to_string()]).await.map_err(storage_error)?;
            let row=rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?;
            Ok(json!({"terminal_id":id.to_string(),"operation_id":row.get::<String>(0).map_err(storage_error)?,"status":row.get::<String>(1).map_err(storage_error)?,"exit_code":row.get::<Option<i64>>(2).map_err(storage_error)?}))
        }).await
    }
    async fn state(&self, workspace: WorkspaceId, id: TerminalId) -> Result<Arc<Live>> {
        let live = self
            .live
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(PortError::NotFound)?;
        if live.workspace != workspace {
            return Err(PortError::NotFound);
        }
        live.last_used.store(
            runtime_protocol::now_ms(),
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(live)
    }
    pub async fn open(&self, input: &Value, context: &ToolContext) -> Result<Value> {
        if !self.config.enabled {
            return Err(PortError::Policy(
                "interactive terminal disabled; local approval required".into(),
            ));
        }
        let guardian = self
            .guardian
            .clone()
            .ok_or_else(|| PortError::Policy("terminal lifecycle guardian missing".into()))?;
        let program = self
            .config
            .program
            .as_ref()
            .ok_or(PortError::NotFound)?
            .canonicalize()
            .map_err(crate::files::io_error)?;
        #[cfg(windows)]
        let program = windows_console_path(program);
        let fingerprint = self.config.fingerprint.clone().ok_or(PortError::NotFound)?;
        let args = self.config.args.clone();
        let geometry = size(input)?;
        let cwd_str = input["cwd"].as_str().unwrap_or(".");
        let cwd = if self.files.limits.scope == crate::files::AccessScope::FullComputer {
            let p = std::path::Path::new(cwd_str);
            let target = if p.is_absolute()
                || (cwd_str.len() >= 2 && cwd_str.as_bytes()[1] == b':')
            {
                PathBuf::from(cwd_str)
            } else {
                self.files.root().join(cwd_str)
            };
            let canon = target.canonicalize().map_err(crate::files::io_error)?;
            if self.files.is_enclave_path(&canon) {
                return Err(PortError::Policy(
                    "terminal cwd access to sensitive enclave denied".into(),
                ));
            }
            if !canon.is_dir() {
                return Err(PortError::Policy("terminal cwd must be a directory".into()));
            }
            canon
        } else {
            let cwd = self.files.validate(cwd_str)?;
            let cwd = self
                .files
                .root()
                .join(cwd)
                .canonicalize()
                .map_err(crate::files::io_error)?;
            if !cwd.starts_with(self.files.root()) || !cwd.is_dir() {
                return Err(PortError::Policy("terminal cwd escapes workspace".into()));
            }
            cwd
        };
        #[cfg(windows)]
        let cwd = {
            use std::path::{Component, Prefix};
            let mut parts = cwd.components();
            match parts.next() {
                Some(Component::Prefix(prefix)) => match prefix.kind() {
                    Prefix::VerbatimDisk(drive) => {
                        let mut normal = PathBuf::from(format!("{}:\\", char::from(drive)));
                        for part in parts {
                            if !matches!(part, Component::RootDir) {
                                normal.push(part.as_os_str());
                            }
                        }
                        normal
                    }
                    Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)
                        if program
                            .file_name()
                            .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe")) =>
                    {
                        return Err(PortError::Policy("cmd.exe cannot use UNC working directories; approve PowerShell or select a local folder".into()));
                    }
                    _ => cwd,
                },
                _ => cwd,
            }
        };
        let mut admission = self.live.write().await;
        if admission.len() >= self.config.max_terminals {
            return Err(PortError::Busy);
        }
        let id: TerminalId = context
            .operation_id
            .to_string()
            .parse()
            .map_err(pty_error)?;
        let spool = self.spool.clone();
        let parent = std::process::id();
        let opened = tokio::task::spawn_blocking(move || {
            if ProgramRule::fingerprint(&program)? != fingerprint {
                return Err(PortError::Policy(
                    "approved terminal fingerprint changed".into(),
                ));
            }
            let mut command = CommandBuilder::new(guardian);
            command.arg("--terminal-guardian");
            command.arg(parent.to_string());
            command.arg(&program);
            command.arg(&fingerprint);
            command.args(&args);
            command.cwd(cwd);
            command.env_clear();
            #[cfg(windows)]
            for key in [
                "SystemRoot",
                "COMSPEC",
                "PATH",
                "TEMP",
                "TMP",
                "USERPROFILE",
                "HOMEDRIVE",
                "HOMEPATH",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
            #[cfg(unix)]
            {
                command.env("TERM", "xterm-256color");
            }
            let pair = native_pty_system().openpty(geometry).map_err(pty_error)?;
            let reader = pair.master.try_clone_reader().map_err(pty_error)?;
            let writer = pair.master.take_writer().map_err(pty_error)?;
            let capture = spool
                .create(&id.to_string())
                .map_err(crate::files::io_error)?;
            let child = pair.slave.spawn_command(command).map_err(pty_error)?;
            drop(pair.slave);
            let pid = child
                .process_id()
                .ok_or_else(|| PortError::Tool("PTY child has no process ID".into()))?;
            Ok::<_, PortError>((pair.master, child, reader, writer, capture, pid))
        })
        .await
        .map_err(storage_error)??;
        let (master, mut child, reader, writer, capture, pid) = opened;
        let (tx, rx) = std::sync::mpsc::sync_channel::<Input>(8);
        let live = Arc::new(Live {
            workspace: context.workspace_id,
            master: Mutex::new(Some(master)),
            killer: Mutex::new(child.clone_killer()),
            writer: tx,
            last_used: std::sync::atomic::AtomicU64::new(runtime_protocol::now_ms()),
            #[cfg(unix)]
            group: pid as i32,
        });
        if context.cancellation.is_cancelled() {
            let _ = live.terminate();
            return Err(PortError::Cancelled);
        }
        let workspace = context.workspace_id;
        let operation = context.operation_id;
        let persisted = self
            .store
            .run(move |c| async move {
                c.execute(
                    "INSERT INTO terminals VALUES(?1,?2,?3,'Running',NULL,?4)",
                    libsql::params![
                        id.to_string(),
                        workspace.to_string(),
                        operation.to_string(),
                        runtime_protocol::now_ms() as i64
                    ],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await;
        if let Err(error) = persisted {
            let _ = live.terminate();
            return Err(error);
        }
        admission.insert(id, live.clone());
        drop(admission);
        std::thread::spawn(move || {
            let mut writer = writer;
            while let Ok((data, reply)) = rx.recv() {
                let result = writer
                    .write_all(&data)
                    .and_then(|_| writer.flush())
                    .map_err(crate::files::io_error);
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let capture = Arc::new(Mutex::new(capture));
        let reader_capture = capture.clone();
        let (done_send, done_recv) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut bytes = [0u8; 8192];
            let result = (|| -> std::io::Result<()> {
                loop {
                    let count = match reader.read(&mut bytes) {
                        Ok(count) => count,
                        // Unix PTYs return EIO when the slave has closed.
                        #[cfg(unix)]
                        Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                        Err(error) => return Err(error),
                    };
                    if count == 0 {
                        break;
                    }
                    reader_capture
                        .lock()
                        .map_err(|_| std::io::Error::other("terminal spool poisoned"))?
                        .append(false, &bytes[..count])?;
                }
                Ok(())
            })();
            let _ = done_send.send(result);
        });
        let store = self.store.clone();
        let states = self.live.clone();
        let runtime = tokio::runtime::Handle::current();
        let timeout = self.config.idle_timeout_seconds;
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Err(error) => break Err(error),
                    _ => {}
                }
                if runtime_protocol::now_ms()
                    .saturating_sub(live.last_used.load(std::sync::atomic::Ordering::Relaxed))
                    > timeout * 1000
                {
                    let _ = live.terminate();
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            };
            // Close ConPTY before waiting for its output pipe to reach EOF.
            if let Ok(mut master) = live.master.lock() {
                master.take();
            }
            let drained = done_recv
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok_and(|result| result.is_ok());
            let captured = capture.lock().ok().is_some_and(|mut capture| {
                capture
                    .finish(started.elapsed().as_millis() as u64, !drained)
                    .is_ok()
            });
            let exit = status.as_ref().ok().map(|s| s.exit_code() as i64);
            let completed = if drained && captured && status.is_ok() {
                "Exited"
            } else {
                "CaptureFailed"
            };
            runtime.block_on(async {
                let _ = store
                    .run(move |c| async move {
                        c.execute(
                            "UPDATE terminals SET status=?2,exit_code=?3 WHERE id=?1",
                            libsql::params![id.to_string(), completed, exit],
                        )
                        .await
                        .map_err(storage_error)?;
                        Ok(())
                    })
                    .await;
                states.write().await.remove(&id);
            });
        });
        if context.cancellation.is_cancelled() {
            let live = self.state(workspace, id).await?;
            let _ = live.terminate();
            return Err(PortError::Cancelled);
        }
        Ok(
            json!({"terminal_id":id.to_string(),"status":"Running","pid":pid,"rows":geometry.rows,"cols":geometry.cols,"stream":"pty","execution_profile":"trusted_interactive","raw_ref":format!("process-spool/{id}/stdout")}),
        )
    }
    pub async fn write(
        &self,
        workspace: WorkspaceId,
        input: &Value,
        context: &ToolContext,
    ) -> Result<Value> {
        let id = terminal_id(input)?;
        let live = self.state(workspace, id).await?;
        let data = input["text"]
            .as_str()
            .ok_or_else(|| PortError::Tool("required terminal text".into()))?
            .as_bytes()
            .to_vec();
        if data.len() > self.config.max_input_bytes {
            return Err(PortError::Policy("terminal input quota exceeded".into()));
        }
        let count = data.len();
        let (tx, rx) = oneshot::channel();
        live.writer
            .try_send((data, tx))
            .map_err(|_| PortError::Busy)?;
        tokio::select! {biased;result=rx=>{result.map_err(pty_error)??;Ok(json!({"terminal_id":id.to_string(),"written_bytes":count}))},_=context.cancellation.cancelled()=>Err(PortError::Cancelled)}
    }
    pub async fn read(&self, workspace: WorkspaceId, input: &Value) -> Result<Value> {
        let id = terminal_id(input)?;
        let status = self.owned(workspace, id).await?;
        if let Some(live) = self.live.read().await.get(&id) {
            live.last_used.store(
                runtime_protocol::now_ms(),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        let offset = input["offset"].as_u64().unwrap_or(0);
        let capacity = (input["max_bytes"].as_u64().unwrap_or(65536) as usize).clamp(1, 131072);
        let spool = self.spool.clone();
        let range = tokio::task::spawn_blocking(move || {
            spool.read_range(&id.to_string(), false, offset, capacity)
        })
        .await
        .map_err(storage_error)?
        .map_err(crate::files::io_error)?;
        let active = status["status"] == "Running";
        let incomplete = range.metadata.capture_incomplete
            || status["status"] == "interrupted"
            || status["status"] == "CaptureFailed";
        let next = range.next_offset.or(if active {
            Some((offset + range.output.len() as u64).min(range.metadata.stdout_stored))
        } else {
            None
        });
        Ok(
            json!({"terminal_id":id.to_string(),"status":status["status"],"exit_code":status["exit_code"],"stream":"pty","offset":offset,"next_offset":next,"encoding":"hex","data":hex(&range.output),"text_encoding":"utf-8-lossy","text":String::from_utf8_lossy(&range.output),"stored_bytes":range.metadata.stdout_stored,"generated_bytes":range.metadata.stdout_bytes,"truncated":range.metadata.truncated || range.next_offset.is_some(),"incomplete":incomplete || active,"capture_incomplete":incomplete,"raw_ref":format!("process-spool/{id}/stdout")}),
        )
    }
    pub async fn resize(&self, workspace: WorkspaceId, input: &Value) -> Result<Value> {
        let id = terminal_id(input)?;
        let live = self.state(workspace, id).await?;
        let geometry = size(input)?;
        tokio::task::spawn_blocking(move || {
            live.master
                .lock()
                .map_err(|_| pty_error("PTY poisoned"))?
                .as_ref()
                .ok_or(PortError::NotFound)?
                .resize(geometry)
                .map_err(pty_error)
        })
        .await
        .map_err(storage_error)??;
        Ok(json!({"terminal_id":id.to_string(),"rows":geometry.rows,"cols":geometry.cols}))
    }
    pub async fn close(&self, workspace: WorkspaceId, input: &Value) -> Result<Value> {
        let id = terminal_id(input)?;
        let status = self.owned(workspace, id).await?;
        if status["status"] != "Running" {
            return Ok(json!({"terminal_id":id.to_string(),"closed":true}));
        }
        let live = self.state(workspace, id).await?;
        tokio::task::spawn_blocking(move || live.terminate())
            .await
            .map_err(storage_error)??;
        Ok(json!({"terminal_id":id.to_string(),"close_requested":true}))
    }
    pub async fn shutdown(&self) {
        for live in self.live.read().await.values() {
            let _ = live.terminate();
        }
        for _ in 0..100 {
            if self.live.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

#[cfg(windows)]
fn windows_console_path(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    if let Some(Component::Prefix(prefix)) = parts.next()
        && let Prefix::VerbatimDisk(drive) = prefix.kind()
    {
        let mut normal = PathBuf::from(format!("{}:\\", char::from(drive)));
        for part in parts {
            if !matches!(part, Component::RootDir) {
                normal.push(part.as_os_str());
            }
        }
        return normal;
    }
    path
}
