//! Handle-relative filesystem confinement. Disk work runs on blocking workers.
use async_trait::async_trait;
use cap_std::fs::{Dir, OpenOptions};
use runtime_domain::FileSnapshot;
use runtime_ports::*;
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileLimits {
    pub max_file_bytes: usize,
    pub max_batch_bytes: usize,
    pub max_batch_files: usize,
    pub max_scan_entries: usize,
    pub excluded: Vec<String>,
}
impl Default for FileLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            max_batch_bytes: 32 * 1024 * 1024,
            max_batch_files: 10_000,
            max_scan_entries: 100_000,
            excluded: ["node_modules", "target", ".git", "build", "dist"]
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }
}
#[derive(Clone)]
pub struct WorkspaceFiles {
    dir: Arc<Dir>,
    root: PathBuf,
    pub limits: FileLimits,
    pub(crate) gate: Arc<RwLock<()>>,
}
pub(crate) fn io_error(e: std::io::Error) -> PortError {
    if e.kind() == std::io::ErrorKind::NotFound {
        PortError::NotFound
    } else {
        PortError::Tool(e.to_string())
    }
}
impl WorkspaceFiles {
    pub fn open(root: &Path, limits: FileLimits) -> Result<Self> {
        let root = root.canonicalize().map_err(io_error)?;
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(io_error)?;
        Ok(Self {
            dir: Arc::new(dir),
            root,
            limits,
            gate: Arc::new(RwLock::new(())),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn lexical(&self, path: &str) -> Result<PathBuf> {
        // Reject both separator styles on every platform, including ADS and device names.
        if path.contains(':') || path.contains('\0') || path.starts_with(['/', '\\']) {
            return Err(PortError::Policy("absolute/namespace path denied".into()));
        }
        let normalized = path.replace('\\', "/");
        let mut relative = PathBuf::new();
        for c in Path::new(&normalized).components() {
            match c {
                Component::Normal(s) => {
                    let name = s.to_string_lossy();
                    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
                    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                        || (base.len() == 4
                            && (base.starts_with("COM") || base.starts_with("LPT"))
                            && matches!(base.as_bytes()[3], b'1'..=b'9'));
                    if device
                        || name.ends_with([' ', '.'])
                        || self
                            .limits
                            .excluded
                            .iter()
                            .any(|x| x.to_uppercase() == name.to_uppercase())
                        || name.to_uppercase().starts_with(".RDC-")
                    {
                        return Err(PortError::Policy("excluded or reserved path".into()));
                    }
                    relative.push(s);
                }
                Component::CurDir => {}
                _ => return Err(PortError::Policy("path traversal denied".into())),
            }
        }
        if relative.as_os_str().is_empty() {
            relative.push(".");
        }
        Ok(relative)
    }
    pub(crate) fn validate(&self, path: &str) -> Result<PathBuf> {
        let relative = self.lexical(path)?;
        let mut prefix = PathBuf::new();
        for component in relative.components() {
            prefix.push(component);
            match self.dir.symlink_metadata(&prefix) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(PortError::Policy("symbolic link denied".into()));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(io_error(e)),
            }
        }
        let mut matcher = ignore::gitignore::GitignoreBuilder::new(&self.root);
        let mut ancestors: Vec<_> = relative
            .parent()
            .unwrap_or(Path::new(""))
            .ancestors()
            .collect();
        ancestors.reverse();
        for ancestor in ancestors {
            let ignore_path = ancestor.join(".gitignore");
            match self.dir.open(&ignore_path) {
                Ok(file) => {
                    if file.metadata().map_err(io_error)?.len() > 64 * 1024 {
                        return Err(PortError::Policy("ignore policy exceeds size limit".into()));
                    }
                    let mut text = String::new();
                    file.take(64 * 1024)
                        .read_to_string(&mut text)
                        .map_err(io_error)?;
                    for line in text.lines() {
                        matcher
                            .add_line(Some(self.root.join(&ignore_path)), line)
                            .map_err(|e| PortError::Policy(e.to_string()))?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(e)),
            }
        }
        let matcher = matcher
            .build()
            .map_err(|e| PortError::Policy(e.to_string()))?;
        let is_dir = self
            .dir
            .metadata(&relative)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if matcher
            .matched_path_or_any_parents(self.root.join(&relative), is_dir)
            .is_ignore()
        {
            return Err(PortError::Policy("gitignored path denied".into()));
        }
        Ok(relative)
    }
    pub(crate) fn read_sync(&self, path: &str) -> Result<Vec<u8>> {
        let relative = self.validate(path)?;
        let file = self.dir.open(relative).map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() || metadata.len() > self.limits.max_file_bytes as u64 {
            return Err(PortError::Policy("file size/type limit".into()));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(self.limits.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > self.limits.max_file_bytes {
            return Err(PortError::Policy("file size limit".into()));
        }
        Ok(bytes)
    }
    pub(crate) fn write_sync(&self, path: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.limits.max_file_bytes {
            return Err(PortError::Policy("file size limit".into()));
        }
        let relative = self.validate(path)?;
        if relative == Path::new(".")
            || relative
                .file_name()
                .is_some_and(|s| s.to_string_lossy().to_uppercase() == ".GITIGNORE")
        {
            return Err(PortError::Policy(
                "cannot replace workspace root or ignore policy".into(),
            ));
        }
        let temp = relative
            .parent()
            .unwrap_or(Path::new(""))
            .join(format!(".rdc-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut f = self
                .dir
                .open_with(&temp, OpenOptions::new().write(true).create_new(true))
                .map_err(io_error)?;
            f.write_all(bytes).map_err(io_error)?;
            f.sync_all().map_err(io_error)?;
            drop(f);
            self.validate(path)?;
            self.dir
                .rename(&temp, &self.dir, &relative)
                .map_err(io_error)?;
            #[cfg(unix)]
            self.dir
                .open_dir(relative.parent().unwrap_or(Path::new("")))
                .map_err(io_error)?
                .into_std_file()
                .sync_all()
                .map_err(io_error)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = self.dir.remove_file(&temp);
        }
        result
    }
    pub(crate) fn restore_sync(&self, snapshots: &[FileSnapshot]) -> Result<()> {
        // Restore ignore files last, so an earlier ignore rule cannot block restoration.
        let mut ordered: Vec<_> = snapshots.iter().collect();
        ordered.sort_by_key(|s| s.path.ends_with(".gitignore"));
        for snapshot in ordered {
            match &snapshot.content {
                Some(bytes) => self.write_sync(&snapshot.path, bytes)?,
                None => {
                    let path = self.validate(&snapshot.path)?;
                    match self.dir.remove_file(path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(io_error(e)),
                    }
                }
            }
        }
        Ok(())
    }
    pub async fn stat(&self, path: &str) -> Result<serde_json::Value> {
        let _guard = self.gate.read().await;
        let fs = self.clone();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let relative = fs.validate(&path)?;
            let m = fs.dir.metadata(relative).map_err(io_error)?;
            Ok(serde_json::json!({"path":path,"bytes":m.len(),"directory":m.is_dir()}))
        })
        .await
        .map_err(|e| PortError::Tool(e.to_string()))?
    }
    pub(crate) async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let fs = self.clone();
        tokio::task::spawn_blocking(move || f(fs))
            .await
            .map_err(|e| PortError::Tool(e.to_string()))?
    }
}
#[async_trait]
impl FileSystemPort for WorkspaceFiles {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let _guard = self.gate.read().await;
        let path = path.to_owned();
        self.blocking(move |fs| fs.read_sync(&path)).await
    }
    async fn write_atomic(&self, path: &str, bytes: Vec<u8>) -> Result<()> {
        let _guard = self.gate.write().await;
        let path = path.to_owned();
        self.blocking(move |fs| fs.write_sync(&path, &bytes)).await
    }
    async fn restore(&self, snapshots: Vec<FileSnapshot>) -> Result<()> {
        let _guard = self.gate.write().await;
        self.blocking(move |fs| fs.restore_sync(&snapshots)).await
    }
    async fn scan(
        &self,
        path: &str,
        limit: usize,
        cancellation: CancellationToken,
    ) -> Result<Vec<String>> {
        let _guard = self.gate.read().await;
        let path = path.to_owned();
        self.blocking(move |fs| {
            let relative = fs.validate(&path)?;
            let mut pending = vec![relative];
            let mut files = Vec::new();
            let mut examined = 0;
            let limit = limit.min(fs.limits.max_scan_entries);
            while let Some(path) = pending.pop() {
                if cancellation.is_cancelled() {
                    return Err(PortError::Cancelled);
                }
                for entry in fs.dir.read_dir(&path).map_err(io_error)? {
                    if cancellation.is_cancelled() {
                        return Err(PortError::Cancelled);
                    }
                    let entry = entry.map_err(io_error)?;
                    examined += 1;
                    if examined > fs.limits.max_scan_entries {
                        return Err(PortError::Policy("scan entry limit".into()));
                    }
                    let p = path.join(entry.file_name());
                    let display = p.to_string_lossy().replace('\\', "/");
                    if fs.validate(&display).is_err() {
                        continue;
                    }
                    let kind = entry.file_type().map_err(io_error)?;
                    if kind.is_dir() {
                        pending.push(p);
                    } else if kind.is_file() {
                        files.push(display);
                        if files.len() >= limit {
                            files.sort();
                            return Ok(files);
                        }
                    }
                }
            }
            files.sort();
            Ok(files)
        })
        .await
    }
}
