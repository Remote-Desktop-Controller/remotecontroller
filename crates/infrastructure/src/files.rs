//! Handle-relative filesystem confinement. Disk work runs on blocking workers.
use async_trait::async_trait;
use cap_std::fs::{Dir, OpenOptions};
use runtime_domain::{FilePrecondition, FileSnapshot};
use runtime_ports::*;
use sha2::{Digest, Sha256};
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
    pub(crate) fn snapshot_sync(&self, path: &str) -> Result<FileSnapshot> {
        let relative = self.validate(path)?;
        let mut file = match self.dir.open(&relative) {
            Ok(file) => file.into_std(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FileSnapshot::new(path, None));
            }
            Err(error) => return Err(io_error(error)),
        };
        let stat = file.metadata().map_err(io_error)?;
        if !stat.is_file() || stat.len() > self.limits.max_file_bytes as u64 {
            return Err(PortError::Policy("file size/type limit".into()));
        }
        let metadata = native_metadata::capture(&file).map_err(io_error)?;
        let mut bytes = Vec::with_capacity(stat.len() as usize);
        (&mut file)
            .take(self.limits.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > self.limits.max_file_bytes {
            return Err(PortError::Policy("file size limit".into()));
        }
        let after = native_metadata::capture(&file).map_err(io_error)?;
        if !native_metadata::same_state(&metadata, &after) || stat.len() != bytes.len() as u64 {
            return Err(PortError::Conflict);
        }
        let snapshot = FileSnapshot {
            path: path.to_owned(),
            content: Some(bytes),
            metadata: Some(metadata),
            restore_precondition: None,
        };
        // A second handle-relative read detects writes/replacements during capture.
        self.check_snapshot_sync(&snapshot)?;
        Ok(snapshot)
    }
    pub(crate) fn check_snapshot_sync(&self, snapshot: &FileSnapshot) -> Result<()> {
        let actual = match self.read_sync(&snapshot.path) {
            Ok(bytes) => Some(bytes),
            Err(PortError::NotFound) => None,
            Err(error) => return Err(error),
        };
        if actual != snapshot.content {
            return Err(PortError::Conflict);
        }
        if let Some(expected) = &snapshot.metadata {
            let relative = self.validate(&snapshot.path)?;
            let file = self.dir.open(relative).map_err(io_error)?.into_std();
            let actual = native_metadata::capture(&file).map_err(io_error)?;
            if !native_metadata::same_state(expected, &actual) {
                return Err(PortError::Conflict);
            }
        }
        Ok(())
    }
    pub(crate) fn write_sync(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let original = self.snapshot_sync(path)?;
        self.apply_snapshot_sync(&original, Some(bytes)).map(|_| ())
    }
    /// Apply only while the preparation snapshot still matches. The returned
    /// snapshot is guarded by the intended post-state for safe rollback.
    pub(crate) fn apply_snapshot_sync(
        &self,
        original: &FileSnapshot,
        content: Option<&[u8]>,
    ) -> Result<FileSnapshot> {
        self.check_snapshot_sync(original)?;
        match content {
            Some(bytes) => self.write_checked_sync(original, bytes, original.metadata.as_ref())?,
            None => {
                let relative = self.validate(&original.path)?;
                if relative == Path::new(".")
                    || relative
                        .file_name()
                        .is_some_and(|s| s.to_string_lossy().to_uppercase() == ".GITIGNORE")
                {
                    return Err(PortError::Policy(
                        "cannot delete workspace root or ignore policy".into(),
                    ));
                }
                self.check_snapshot_sync(original)?;
                let parent = relative
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let parent = self.dir.open_dir(parent).map_err(io_error)?;
                let name = relative
                    .file_name()
                    .ok_or_else(|| PortError::Policy("workspace root cannot be deleted".into()))?;
                match parent.remove_file(name) {
                    Ok(()) => {}
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound
                            && original.content.is_none() => {}
                    Err(error) => return Err(io_error(error)),
                }
                #[cfg(unix)]
                parent
                    .open(".")
                    .map_err(io_error)?
                    .sync_all()
                    .map_err(io_error)?;
            }
        }
        let mut rollback = original.clone();
        rollback.restore_precondition = Some(match content {
            Some(bytes) => FilePrecondition::Sha256(Sha256::digest(bytes).into()),
            None => FilePrecondition::Missing,
        });
        Ok(rollback)
    }
    fn write_checked_sync(
        &self,
        expected: &FileSnapshot,
        bytes: &[u8],
        metadata: Option<&runtime_domain::FileMetadata>,
    ) -> Result<()> {
        if bytes.len() > self.limits.max_file_bytes {
            return Err(PortError::Policy("file size limit".into()));
        }
        let relative = self.validate(&expected.path)?;
        if relative == Path::new(".")
            || relative
                .file_name()
                .is_some_and(|s| s.to_string_lossy().to_uppercase() == ".GITIGNORE")
        {
            return Err(PortError::Policy(
                "cannot replace workspace root or ignore policy".into(),
            ));
        }
        let parent_path = relative
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        // Pin the directory handle; all create/rename/remove operations use it.
        let parent = self.dir.open_dir(parent_path).map_err(io_error)?;
        let name = relative
            .file_name()
            .ok_or_else(|| PortError::Policy("missing filename".into()))?;
        let temp = format!(".rdc-{}.tmp", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.access_mode(0x8000_0000 | 0x4000_0000 | 0x0004_0000);
        }
        let mut file = parent
            .open_with(&temp, &options)
            .map_err(io_error)?
            .into_std();
        let result = (|| {
            file.write_all(bytes).map_err(io_error)?;
            if let Some(metadata) = metadata {
                native_metadata::restore(&file, metadata).map_err(io_error)?;
                let restored = native_metadata::capture(&file).map_err(io_error)?;
                if !native_metadata::same_state(metadata, &restored)
                    || metadata.accessed != restored.accessed
                {
                    return Err(PortError::Tool(format!(
                        "metadata restoration verification failed: {}",
                        native_metadata::differences(metadata, &restored)
                    )));
                }
            }
            file.sync_all().map_err(io_error)?;
            self.validate(&expected.path)?;
            self.check_snapshot_sync(expected)?;
            parent.rename(&temp, &parent, name).map_err(io_error)?;
            #[cfg(unix)]
            parent
                .open(".")
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
            Ok(())
        })();
        if result.is_err() {
            #[cfg(windows)]
            if let Ok(metadata) = file.metadata() {
                let mut permissions = metadata.permissions();
                // Windows-only cleanup of our temporary file; Unix mode bits
                // are never broadened by this path.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = file.set_permissions(permissions);
            }
        }
        drop(file);
        if result.is_err() {
            let _ = parent.remove_file(&temp);
        }
        result
    }
    fn restore_plan_sync(
        &self,
        snapshots: &[FileSnapshot],
    ) -> Result<Vec<(FileSnapshot, FileSnapshot)>> {
        // Preflight the whole batch before mutation. A conflicting writer keeps
        // the checkpoint pending rather than silently losing external bytes.
        let mut prepared = Vec::new();
        for snapshot in snapshots {
            let current = self.snapshot_sync(&snapshot.path)?;
            if current.content == snapshot.content {
                if let (Some(a), Some(b)) = (&snapshot.metadata, &current.metadata)
                    && !native_metadata::same_state(a, b)
                {
                    return Err(PortError::Conflict);
                }
                continue;
            }
            let allowed = match &snapshot.restore_precondition {
                Some(FilePrecondition::Missing) => current.content.is_none(),
                Some(FilePrecondition::Sha256(hash)) => current
                    .content
                    .as_ref()
                    .is_some_and(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)) == *hash),
                None => false,
            };
            if !allowed {
                return Err(PortError::Conflict);
            }
            // Applying preserved metadata also permits detection of external chmod.
            if current.content.is_some()
                && snapshot.content.is_some()
                && let (Some(a), Some(b)) = (&snapshot.metadata, &current.metadata)
                && !native_metadata::same_state(a, b)
            {
                return Err(PortError::Conflict);
            }
            prepared.push((snapshot.clone(), current));
        }
        Ok(prepared)
    }
    pub(crate) fn preflight_restore_sync(&self, snapshots: &[FileSnapshot]) -> Result<()> {
        self.restore_plan_sync(snapshots).map(|_| ())
    }
    pub(crate) fn restore_sync(&self, snapshots: &[FileSnapshot]) -> Result<()> {
        let prepared = self.restore_plan_sync(snapshots)?;
        for (snapshot, current) in prepared {
            self.check_snapshot_sync(&current)?;
            match &snapshot.content {
                Some(bytes) => {
                    self.write_checked_sync(&current, bytes, snapshot.metadata.as_ref())?
                }
                None => {
                    self.apply_snapshot_sync(&current, None)?;
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

mod native_metadata {
    use runtime_domain::{FileMetadata, UnixFileMetadata, WindowsFileMetadata};
    use std::{fs::File, io};

    pub(super) fn capture(file: &File) -> io::Result<FileMetadata> {
        let metadata = file.metadata()?;
        Ok(FileMetadata {
            accessed: metadata.accessed()?,
            modified: metadata.modified()?,
            readonly: metadata.permissions().readonly(),
            unix: capture_unix(file)?,
            windows: capture_windows(file)?,
        })
    }
    pub(super) fn restore(file: &File, metadata: &FileMetadata) -> io::Result<()> {
        restore_unix(file, metadata)?;
        restore_windows(file, metadata)?;
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(metadata.accessed)
                .set_modified(metadata.modified),
        )?;
        Ok(())
    }
    /// Reading the source itself changes atime, so it is deliberately excluded
    /// from writer detection. All other supported captured properties are checked.
    pub(super) fn same_state(a: &FileMetadata, b: &FileMetadata) -> bool {
        a.modified == b.modified
            && a.readonly == b.readonly
            && a.unix == b.unix
            && a.windows == b.windows
    }
    pub(super) fn differences(a: &FileMetadata, b: &FileMetadata) -> String {
        let mut differences = Vec::new();
        if a.modified != b.modified {
            differences.push(format!("modified {:?} -> {:?}", a.modified, b.modified));
        }
        if a.accessed != b.accessed {
            differences.push(format!("accessed {:?} -> {:?}", a.accessed, b.accessed));
        }
        if a.readonly != b.readonly {
            differences.push(format!("readonly {} -> {}", a.readonly, b.readonly));
        }
        if a.unix != b.unix {
            differences.push("unix metadata differs".into());
        }
        if a.windows != b.windows {
            if let (Some(a), Some(b)) = (&a.windows, &b.windows) {
                if a.created != b.created {
                    differences.push(format!("created {:?} -> {:?}", a.created, b.created));
                }
                if a.attributes != b.attributes {
                    differences.push(format!(
                        "attributes {:#x} -> {:#x}",
                        a.attributes, b.attributes
                    ));
                }
                if a.dacl_protected != b.dacl_protected {
                    differences.push(format!(
                        "DACL protected {} -> {}",
                        a.dacl_protected, b.dacl_protected
                    ));
                }
                if a.dacl != b.dacl {
                    let control = |d: &[u8]| {
                        if d.len() >= 4 {
                            u16::from_le_bytes([d[2], d[3]])
                        } else {
                            0
                        }
                    };
                    differences.push(format!(
                        "DACL differs (size {} -> {}, control {:#x} -> {:#x})",
                        a.dacl.len(),
                        b.dacl.len(),
                        control(&a.dacl),
                        control(&b.dacl)
                    ));
                }
            } else {
                differences.push("Windows metadata platform differs".into());
            }
        }
        differences.join("; ")
    }
    #[cfg(not(unix))]
    fn capture_unix(_: &File) -> io::Result<Option<UnixFileMetadata>> {
        Ok(None)
    }
    #[cfg(not(unix))]
    fn restore_unix(_: &File, metadata: &FileMetadata) -> io::Result<()> {
        if metadata.unix.is_some() {
            return Err(io::Error::other(
                "Unix metadata cannot be restored on this platform",
            ));
        }
        Ok(())
    }
    #[cfg(unix)]
    fn capture_unix(file: &File) -> io::Result<Option<UnixFileMetadata>> {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        reject_unsupported_unix_flags(file)?;
        Ok(Some(UnixFileMetadata {
            mode: m.mode(),
            uid: m.uid(),
            gid: m.gid(),
            extended_attributes: xattrs(file)?,
            acl: mac_acl(file)?,
        }))
    }
    #[cfg(target_os = "macos")]
    fn reject_unsupported_unix_flags(file: &File) -> io::Result<()> {
        use std::os::macos::fs::MetadataExt;
        if file.metadata()?.st_flags() != 0 {
            return Err(io::Error::other(
                "BSD file flags require unsupported preservation",
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    fn reject_unsupported_unix_flags(file: &File) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let mut flags: libc::c_long = 0;
        // Linux FS_IOC_GETFLAGS uses a native long in its ioctl request ABI.
        let request = (0x8000_6601u64 | ((std::mem::size_of::<libc::c_long>() as u64) << 16))
            as libc::c_ulong;
        if unsafe { libc::ioctl(file.as_raw_fd(), request, &mut flags) } < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOTTY) | Some(libc::EOPNOTSUPP)
            ) {
                return Ok(());
            }
            return Err(error);
        }
        // Allocation-layout flags (e.g. EXTENTS) do not represent access/data
        // policy. User-visible semantic flags (immutable/nodump/etc) do.
        if flags & !(0x0008_0000 | 0x1000_0000 | 0x0000_1000) != 0 {
            return Err(io::Error::other(
                "Linux inode flags require unsupported preservation",
            ));
        }
        Ok(())
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn reject_unsupported_unix_flags(_: &File) -> io::Result<()> {
        Ok(())
    }
    #[cfg(unix)]
    fn restore_unix(file: &File, metadata: &FileMetadata) -> io::Result<()> {
        use std::os::unix::{
            fs::{MetadataExt, PermissionsExt},
            io::AsRawFd,
        };
        if metadata.windows.is_some() {
            return Err(io::Error::other(
                "Windows metadata cannot be restored on this platform",
            ));
        }
        if let Some(m) = &metadata.unix {
            let current = file.metadata()?;
            if (current.uid() != m.uid || current.gid() != m.gid)
                && unsafe { libc::fchown(file.as_raw_fd(), m.uid, m.gid) } != 0
            {
                return Err(io::Error::last_os_error());
            }
            file.set_permissions(std::fs::Permissions::from_mode(m.mode))?;
            for (name, _) in xattrs(file)? {
                if !m
                    .extended_attributes
                    .iter()
                    .any(|(expected, _)| *expected == name)
                {
                    remove_xattr(file, &name)?;
                }
            }
            for (name, value) in &m.extended_attributes {
                set_xattr(file, name, value)?;
            }
            restore_mac_acl(file, m.acl.as_deref())?;
        } else {
            let mut p = file.metadata()?.permissions();
            p.set_readonly(metadata.readonly);
            file.set_permissions(p)?;
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn xattrs(file: &File) -> io::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        #[cfg(target_os = "linux")]
        let size = unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0) };
        #[cfg(target_os = "macos")]
        let size = unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0, 0) };
        if size < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOTSUP) {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        if size > 1024 * 1024 {
            return Err(io::Error::other("extended attribute list exceeds limit"));
        }
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut names = vec![0u8; size as usize];
        #[cfg(target_os = "linux")]
        let read = unsafe { libc::flistxattr(fd, names.as_mut_ptr().cast(), names.len()) };
        #[cfg(target_os = "macos")]
        let read = unsafe { libc::flistxattr(fd, names.as_mut_ptr().cast(), names.len(), 0) };
        if read < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = Vec::new();
        let mut total = 0usize;
        for name in names[..read as usize]
            .split(|b| *b == 0)
            .filter(|name| !name.is_empty())
        {
            let c = std::ffi::CString::new(name).map_err(io::Error::other)?;
            #[cfg(target_os = "linux")]
            let size = unsafe { libc::fgetxattr(fd, c.as_ptr(), std::ptr::null_mut(), 0) };
            #[cfg(target_os = "macos")]
            let size = unsafe { libc::fgetxattr(fd, c.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
            if size < 0 {
                return Err(io::Error::last_os_error());
            }
            total = total
                .checked_add(size as usize)
                .ok_or_else(|| io::Error::other("attribute size overflow"))?;
            if total > 1024 * 1024 {
                return Err(io::Error::other("extended attributes exceed limit"));
            }
            let mut value = vec![0; size as usize];
            #[cfg(target_os = "linux")]
            let read =
                unsafe { libc::fgetxattr(fd, c.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
            #[cfg(target_os = "macos")]
            let read = unsafe {
                libc::fgetxattr(fd, c.as_ptr(), value.as_mut_ptr().cast(), value.len(), 0, 0)
            };
            if read < 0 {
                return Err(io::Error::last_os_error());
            }
            value.truncate(read as usize);
            attributes.push((name.to_vec(), value));
        }
        attributes.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(attributes)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn set_xattr(file: &File, name: &[u8], value: &[u8]) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn remove_xattr(file: &File, name: &[u8]) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        #[cfg(target_os = "linux")]
        let result = unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr()) };
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr(), 0) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn remove_xattr(_: &File, _: &[u8]) -> io::Result<()> {
        Err(io::Error::other("native xattrs unsupported"))
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn xattrs(_: &File) -> io::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Err(io::Error::other(
            "native metadata capture unsupported on this Unix platform",
        ))
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn set_xattr(_: &File, _: &[u8], _: &[u8]) -> io::Result<()> {
        Err(io::Error::other("native xattrs unsupported"))
    }

    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn acl_get_fd(fd: libc::c_int) -> *mut libc::c_void;
        fn acl_set_fd(fd: libc::c_int, acl: *mut libc::c_void) -> libc::c_int;
        fn acl_to_text(acl: *mut libc::c_void, len: *mut libc::ssize_t) -> *mut libc::c_char;
        fn acl_from_text(text: *const libc::c_char) -> *mut libc::c_void;
        fn acl_free(value: *mut libc::c_void) -> libc::c_int;
        fn acl_init(count: libc::c_int) -> *mut libc::c_void;
    }
    #[cfg(target_os = "macos")]
    fn mac_acl(file: &File) -> io::Result<Option<Vec<u8>>> {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let acl = acl_get_fd(file.as_raw_fd());
            if acl.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ENOENT) {
                    return Ok(None);
                }
                return Err(error);
            }
            let mut len = 0;
            let text = acl_to_text(acl, &mut len);
            acl_free(acl);
            if text.is_null() {
                return Err(io::Error::last_os_error());
            }
            let bytes = std::ffi::CStr::from_ptr(text).to_bytes().to_vec();
            acl_free(text.cast());
            if bytes.len() > 1024 * 1024 {
                return Err(io::Error::other("ACL exceeds limit"));
            }
            // An absent ACL and a native empty ACL have the same permissions.
            // Canonicalize both so clearing an inherited ACL verifies correctly.
            if bytes
                .split(|b| *b == b'\n')
                .all(|line| line.is_empty() || line.starts_with(b"!#acl"))
            {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    fn mac_acl(_: &File) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    #[cfg(target_os = "macos")]
    fn restore_mac_acl(file: &File, acl: Option<&[u8]>) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        if let Some(text) = acl {
            let text = std::ffi::CString::new(text).map_err(io::Error::other)?;
            unsafe {
                let acl = acl_from_text(text.as_ptr());
                if acl.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let result = acl_set_fd(file.as_raw_fd(), acl);
                let error = io::Error::last_os_error();
                acl_free(acl);
                if result != 0 {
                    return Err(error);
                }
            }
        } else {
            unsafe {
                let empty = acl_init(0);
                if empty.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let result = acl_set_fd(file.as_raw_fd(), empty);
                let error = io::Error::last_os_error();
                acl_free(empty);
                if result != 0 {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    fn restore_mac_acl(_: &File, acl: Option<&[u8]>) -> io::Result<()> {
        if acl.is_some() {
            return Err(io::Error::other(
                "macOS ACL cannot be restored on this platform",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fn capture_windows(_: &File) -> io::Result<Option<WindowsFileMetadata>> {
        Ok(None)
    }
    #[cfg(not(windows))]
    fn restore_windows(_: &File, metadata: &FileMetadata) -> io::Result<()> {
        if metadata.windows.is_some() {
            return Err(io::Error::other(
                "Windows metadata cannot be restored on this platform",
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    #[repr(C)]
    struct BasicInfo {
        creation: i64,
        access: i64,
        write: i64,
        change: i64,
        attributes: u32,
    }
    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    #[cfg(windows)]
    fn capture_windows(file: &File) -> io::Result<Option<WindowsFileMetadata>> {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        use windows_sys::Win32::Security::*;
        let metadata = file.metadata()?;
        let attributes = metadata.file_attributes();
        ensure_default_stream_only(file)?;
        // Basic replacement cannot retain compression, sparse allocation,
        // EFS encryption, reparse points or offline data. Refuse before mutation.
        if attributes & !(0x1 | 0x2 | 0x4 | 0x20 | 0x80 | 0x100) != 0 {
            return Err(io::Error::other(
                "Windows file attributes require unsupported preservation",
            ));
        }
        let mut needed = 0;
        unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        if needed == 0 || needed > 1024 * 1024 {
            return Err(io::Error::last_os_error());
        }
        let mut dacl = vec![0u8; needed as usize];
        if unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION,
                dacl.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut control = 0;
        let mut revision = 0;
        if unsafe {
            GetSecurityDescriptorControl(dacl.as_mut_ptr().cast(), &mut control, &mut revision)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(WindowsFileMetadata {
            created: metadata.created()?,
            attributes,
            dacl,
            dacl_protected: control & SE_DACL_PROTECTED != 0,
        }))
    }
    #[cfg(windows)]
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    #[cfg(windows)]
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryInformationFile(
            handle: *mut std::ffi::c_void,
            status: *mut IoStatus,
            info: *mut std::ffi::c_void,
            length: u32,
            class: u32,
        ) -> i32;
    }
    #[cfg(windows)]
    fn ensure_default_stream_only(file: &File) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        // FileStreamInformation enumerates streams on the already-confined handle.
        // Named streams cannot be silently discarded by atomic replacement.
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut buffer = vec![0u64; 8192];
        let result = unsafe {
            NtQueryInformationFile(
                file.as_raw_handle(),
                &mut status,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                22,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "cannot safely enumerate file streams: NTSTATUS {result:#x}"
            )));
        }
        let raw = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>(),
                status.information.min(buffer.len() * 8),
            )
        };
        let mut offset = 0usize;
        while offset < raw.len() {
            if raw.len() - offset < 24 {
                return Err(io::Error::other("invalid native stream metadata"));
            }
            let next = u32::from_ne_bytes(raw[offset..offset + 4].try_into().unwrap()) as usize;
            let length =
                u32::from_ne_bytes(raw[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let end = offset
                .checked_add(24)
                .and_then(|start| start.checked_add(length))
                .ok_or_else(|| io::Error::other("stream metadata overflow"))?;
            if end > raw.len() || !length.is_multiple_of(2) {
                return Err(io::Error::other("invalid native stream name"));
            }
            let name: Vec<u16> = raw[offset + 24..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_ne_bytes([b[0], b[1]]))
                .collect();
            if String::from_utf16_lossy(&name) != "::$DATA" {
                return Err(io::Error::other(
                    "named Windows streams require unsupported preservation",
                ));
            }
            if next == 0 {
                break;
            }
            if next < 24
                || offset
                    .checked_add(next)
                    .is_none_or(|next| next >= raw.len())
            {
                return Err(io::Error::other("invalid native stream chain"));
            }
            offset += next;
        }
        Ok(())
    }
    #[cfg(windows)]
    fn windows_ticks(time: std::time::SystemTime) -> io::Result<i64> {
        const EPOCH: i128 = 116_444_736_000_000_000;
        let delta = match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => (d.as_nanos() / 100) as i128,
            Err(e) => -((e.duration().as_nanos() / 100) as i128),
        };
        i64::try_from(EPOCH + delta).map_err(io::Error::other)
    }
    #[cfg(windows)]
    fn validate_dacl_descriptor(bytes: &[u8]) -> io::Result<()> {
        // Validate self-relative offsets before passing a persisted descriptor
        // to native code. This representation intentionally contains only DACL.
        if bytes.len() < 20 || bytes.len() > 1024 * 1024 || bytes[0] != 1 {
            return Err(io::Error::other("invalid persisted DACL descriptor"));
        }
        let control = u16::from_le_bytes([bytes[2], bytes[3]]);
        if control & 0x8000 == 0 || bytes[4..16].iter().any(|b| *b != 0) {
            return Err(io::Error::other(
                "unsupported persisted security descriptor",
            ));
        }
        let offset = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        if offset == 0 {
            return Ok(());
        } // Native NULL DACL is retained exactly.
        if offset < 20
            || !offset.is_multiple_of(4)
            || offset.checked_add(8).is_none_or(|end| end > bytes.len())
        {
            return Err(io::Error::other("invalid persisted DACL offset"));
        }
        let size = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if size < 8 || offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err(io::Error::other("invalid persisted DACL size"));
        }
        let count = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]) as usize;
        let mut entry = offset + 8;
        for _ in 0..count {
            if entry.checked_add(4).is_none_or(|end| end > offset + size) {
                return Err(io::Error::other("invalid persisted ACE"));
            }
            let length = u16::from_le_bytes([bytes[entry + 2], bytes[entry + 3]]) as usize;
            if length < 4
                || entry
                    .checked_add(length)
                    .is_none_or(|end| end > offset + size)
            {
                return Err(io::Error::other("invalid persisted ACE size"));
            }
            entry += length;
        }
        Ok(())
    }
    #[cfg(windows)]
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn SetSecurityInfo(
            handle: *mut std::ffi::c_void,
            object_type: i32,
            security_info: u32,
            owner: *const std::ffi::c_void,
            group: *const std::ffi::c_void,
            dacl: *const std::ffi::c_void,
            sacl: *const std::ffi::c_void,
        ) -> u32;
    }
    #[cfg(windows)]
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSetSecurityObject(
            handle: *mut std::ffi::c_void,
            security_info: u32,
            descriptor: *const std::ffi::c_void,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    #[cfg(windows)]
    pub(super) fn set_windows_dacl(file: &File, metadata: &WindowsFileMetadata) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Security::*;
        validate_dacl_descriptor(&metadata.dacl)?;
        let control = u16::from_le_bytes([metadata.dacl[2], metadata.dacl[3]]);
        if control & 0x0400 == 0 && !metadata.dacl_protected {
            // SetSecurityInfo converts legacy ACLs to auto-inheritance and may
            // append parent ACEs. Restore the already captured descriptor on
            // the pinned file handle, with the same WRITE_DAC access check.
            let status = unsafe {
                NtSetSecurityObject(
                    file.as_raw_handle(),
                    DACL_SECURITY_INFORMATION,
                    metadata.dacl.as_ptr().cast(),
                )
            };
            if status < 0 {
                return Err(io::Error::from_raw_os_error(
                    unsafe { RtlNtStatusToDosError(status) } as i32,
                ));
            }
            return Ok(());
        }
        let offset = u32::from_le_bytes(metadata.dacl[16..20].try_into().unwrap()) as usize;
        let dacl = if offset == 0 {
            std::ptr::null()
        } else {
            unsafe { metadata.dacl.as_ptr().add(offset).cast() }
        };
        let protection = if metadata.dacl_protected {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let result = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                1,
                DACL_SECURITY_INFORMATION | protection,
                std::ptr::null(),
                std::ptr::null(),
                dacl,
                std::ptr::null(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        Ok(())
    }
    #[cfg(windows)]
    fn restore_windows(file: &File, metadata: &FileMetadata) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        if metadata.unix.is_some() {
            return Err(io::Error::other(
                "Unix metadata cannot be restored on this platform",
            ));
        }
        if let Some(m) = &metadata.windows {
            set_windows_dacl(file, m)?;
            let info = BasicInfo {
                creation: windows_ticks(m.created)?,
                access: windows_ticks(metadata.accessed)?,
                write: windows_ticks(metadata.modified)?,
                change: 0,
                attributes: m.attributes,
            };
            if unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    0,
                    (&info as *const BasicInfo).cast(),
                    std::mem::size_of::<BasicInfo>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
        } else {
            let mut p = file.metadata()?.permissions();
            p.set_readonly(metadata.readonly);
            file.set_permissions(p)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod integrity_tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, WorkspaceFiles) {
        let root = tempfile::tempdir().unwrap();
        let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
        (root, files)
    }
    #[test]
    fn preparation_rejects_external_binary_writer() {
        let (root, files) = workspace();
        std::fs::write(root.path().join("binary"), [0, 255, 128]).unwrap();
        let before = files.snapshot_sync("binary").unwrap();
        std::fs::write(root.path().join("binary"), [9, 0, 255]).unwrap();
        assert!(matches!(
            files.apply_snapshot_sync(&before, Some(&[1, 2])),
            Err(PortError::Conflict)
        ));
        assert_eq!(
            std::fs::read(root.path().join("binary")).unwrap(),
            [9, 0, 255]
        );
    }
    #[test]
    fn guarded_binary_rollback_preserves_content_and_metadata() {
        let (root, files) = workspace();
        let path = root.path().join("binary");
        std::fs::write(&path, [0, 255, 128]).unwrap();
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        let before = files.snapshot_sync("binary").unwrap();
        let rollback = files
            .apply_snapshot_sync(&before, Some(&[255, 1, 0]))
            .unwrap();
        files.restore_sync(&[rollback]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 128]);
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            timestamp
        );
    }
    #[test]
    fn rollback_rejects_external_writer_without_mutating_other_files() {
        let (root, files) = workspace();
        std::fs::write(root.path().join("first"), [0, 255]).unwrap();
        std::fs::write(root.path().join("second"), [128, 0]).unwrap();
        let first = files.snapshot_sync("first").unwrap();
        let second = files.snapshot_sync("second").unwrap();
        let first = files.apply_snapshot_sync(&first, Some(&[2, 3])).unwrap();
        let second = files.apply_snapshot_sync(&second, Some(&[4, 5])).unwrap();
        std::fs::write(root.path().join("second"), [9, 255]).unwrap();
        assert!(matches!(
            files.restore_sync(&[first, second]),
            Err(PortError::Conflict)
        ));
        assert_eq!(std::fs::read(root.path().join("first")).unwrap(), [2, 3]);
        assert_eq!(std::fs::read(root.path().join("second")).unwrap(), [9, 255]);
    }
    #[test]
    fn guarded_deletion_restores_binary_and_timestamp() {
        let (root, files) = workspace();
        let path = root.path().join("binary");
        std::fs::write(&path, [0, 255, 128]).unwrap();
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        let before = files.snapshot_sync("binary").unwrap();
        let rollback = files.apply_snapshot_sync(&before, None).unwrap();
        assert!(!path.exists());
        files.restore_sync(&[rollback]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 128]);
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            timestamp
        );
    }
    #[cfg(windows)]
    #[test]
    fn legacy_unprotected_dacl_survives_without_added_parent_aces() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, SetFileSecurityW};
        let (root, files) = workspace();
        let path = root.path().join("legacy-acl");
        std::fs::write(&path, b"original").unwrap();
        let before = files.snapshot_sync("legacy-acl").unwrap();
        let mut descriptor = before.metadata.unwrap().windows.unwrap().dacl;
        let mut control = u16::from_le_bytes([descriptor[2], descriptor[3]]);
        control &= !(0x0100 | 0x0400 | 0x1000);
        descriptor[2..4].copy_from_slice(&control.to_le_bytes());
        let offset = u32::from_le_bytes(descriptor[16..20].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes([descriptor[offset + 4], descriptor[offset + 5]]) as usize;
        let mut entry = offset + 8;
        for _ in 0..count {
            descriptor[entry + 1] &= !0x10;
            entry += u16::from_le_bytes([descriptor[entry + 2], descriptor[entry + 3]]) as usize;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // Build a real legacy descriptor on the fixture, not a mocked snapshot.
        assert_ne!(
            unsafe {
                SetFileSecurityW(
                    wide.as_ptr(),
                    DACL_SECURITY_INFORMATION,
                    descriptor.as_mut_ptr().cast(),
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        let original = files.snapshot_sync("legacy-acl").unwrap();
        let dacl = &original
            .metadata
            .as_ref()
            .unwrap()
            .windows
            .as_ref()
            .unwrap()
            .dacl;
        assert_eq!(
            u16::from_le_bytes([dacl[2], dacl[3]]) & 0x0400,
            0,
            "fixture must retain legacy inheritance model"
        );
        files
            .apply_snapshot_sync(&original, Some(b"changed"))
            .unwrap();
        let after = files.snapshot_sync("legacy-acl").unwrap();
        assert!(native_metadata::same_state(
            original.metadata.as_ref().unwrap(),
            after.metadata.as_ref().unwrap()
        ));
    }
    #[cfg(windows)]
    #[test]
    fn hidden_attribute_and_native_dacl_survive_atomic_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let (root, files) = workspace();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .access_mode(0x8000_0000 | 0x4000_0000 | 0x0004_0000)
            .attributes(0x2)
            .open(root.path().join("hidden"))
            .unwrap();
        file.write_all(&[0, 255]).unwrap();
        let metadata = native_metadata::capture(&file).unwrap();
        let mut native = metadata.windows.unwrap();
        native.dacl_protected = true;
        native_metadata::set_windows_dacl(&file, &native).unwrap();
        drop(file);
        let before = files.snapshot_sync("hidden").unwrap();
        assert_ne!(
            before
                .metadata
                .as_ref()
                .unwrap()
                .windows
                .as_ref()
                .unwrap()
                .attributes
                & 0x2,
            0
        );
        assert!(
            before
                .metadata
                .as_ref()
                .unwrap()
                .windows
                .as_ref()
                .unwrap()
                .dacl_protected
        );
        files.apply_snapshot_sync(&before, Some(&[255, 0])).unwrap();
        let after = files.snapshot_sync("hidden").unwrap();
        assert_eq!(after.content, Some(vec![255, 0]));
        assert!(native_metadata::same_state(
            before.metadata.as_ref().unwrap(),
            after.metadata.as_ref().unwrap()
        ));
    }
    #[cfg(windows)]
    #[test]
    fn named_stream_is_rejected_without_losing_either_stream() {
        let (root, files) = workspace();
        std::fs::write(root.path().join("streams"), [0, 255]).unwrap();
        std::fs::write(root.path().join("streams:private"), [128, 1]).unwrap();
        assert!(files.write_sync("streams", &[3]).is_err());
        assert_eq!(
            std::fs::read(root.path().join("streams")).unwrap(),
            [0, 255]
        );
        assert_eq!(
            std::fs::read(root.path().join("streams:private")).unwrap(),
            [128, 1]
        );
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn native_extended_attribute_survives_replacement() {
        use std::os::unix::io::AsRawFd;
        let (root, files) = workspace();
        let path = root.path().join("attributes");
        std::fs::write(&path, [0, 255]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let name = std::ffi::CString::new("user.rdc-test").unwrap();
        let value = [255u8, 0, 128];
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        drop(file);
        let before = files.snapshot_sync("attributes").unwrap();
        files.apply_snapshot_sync(&before, Some(&[255, 0])).unwrap();
        let after = files.snapshot_sync("attributes").unwrap();
        assert!(
            after
                .metadata
                .as_ref()
                .unwrap()
                .unix
                .as_ref()
                .unwrap()
                .extended_attributes
                .iter()
                .any(|(n, v)| *n == b"user.rdc-test" && *v == value)
        );
    }
}

#[cfg(unix)]
#[test]
fn rollback_rejects_external_permission_change() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("binary");
    std::fs::write(&path, [0, 255]).unwrap();
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let before = files.snapshot_sync("binary").unwrap();
    let rollback = files.apply_snapshot_sync(&before, Some(&[128, 0])).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        files.restore_sync(&[rollback]),
        Err(PortError::Conflict)
    ));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(std::fs::read(path).unwrap(), [128, 0]);
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn linux_posix_acl_survives_atomic_replacement() {
    use std::os::unix::io::AsRawFd;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("acl");
    std::fs::write(&path, [0, 255]).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let name = std::ffi::CString::new("system.posix_acl_access").unwrap();
    // Linux native ACL xattr v2: owner, named user, group, mask, other.
    let value: [u8; 44] = [
        2, 0, 0, 0, 1, 0, 7, 0, 255, 255, 255, 255, 2, 0, 4, 0, 57, 48, 0, 0, 4, 0, 4, 0, 255, 255,
        255, 255, 16, 0, 4, 0, 255, 255, 255, 255, 32, 0, 0, 0, 255, 255, 255, 255,
    ];
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    drop(file);
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let before = files.snapshot_sync("acl").unwrap();
    files.apply_snapshot_sync(&before, Some(&[255, 0])).unwrap();
    let after = files.snapshot_sync("acl").unwrap();
    assert_eq!(
        after
            .metadata
            .as_ref()
            .unwrap()
            .unix
            .as_ref()
            .unwrap()
            .extended_attributes,
        before
            .metadata
            .as_ref()
            .unwrap()
            .unix
            .as_ref()
            .unwrap()
            .extended_attributes
    );
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_extended_acl_survives_atomic_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("acl");
    std::fs::write(&path, [0, 255]).unwrap();
    assert!(
        std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read"])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let before = files.snapshot_sync("acl").unwrap();
    assert!(
        before
            .metadata
            .as_ref()
            .unwrap()
            .unix
            .as_ref()
            .unwrap()
            .acl
            .is_some()
    );
    files.apply_snapshot_sync(&before, Some(&[255, 0])).unwrap();
    let after = files.snapshot_sync("acl").unwrap();
    assert_eq!(
        after.metadata.as_ref().unwrap().unix.as_ref().unwrap().acl,
        before.metadata.as_ref().unwrap().unix.as_ref().unwrap().acl
    );
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn unsupported_linux_inode_flag_is_refused_before_mutation() {
    use std::os::unix::io::AsRawFd;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("flagged");
    std::fs::write(&path, [0, 255]).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let size = (std::mem::size_of::<libc::c_long>() as u64) << 16;
    let get = (0x8000_6601u64 | size) as libc::c_ulong;
    let set = (0x4000_6602u64 | size) as libc::c_ulong;
    let mut flags: libc::c_long = 0;
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), get, &mut flags) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    flags |= 0x40; // NODUMP is user-settable; preservation is explicitly unsupported.
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), set, &flags) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    assert!(files.write_sync("flagged", &[128]).is_err());
    assert_eq!(std::fs::read(path).unwrap(), [0, 255]);
    let mut after: libc::c_long = 0;
    assert_eq!(unsafe { libc::ioctl(file.as_raw_fd(), get, &mut after) }, 0);
    assert_ne!(after & 0x40, 0);
}
