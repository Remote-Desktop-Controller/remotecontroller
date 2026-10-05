use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ipc.secret must be a regular private file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ipc.secret must be owned by the current user with mode 0600",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ipc.secret cannot be a reparse point",
            ));
        }
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(33));
    file.take(33).read_to_end(&mut bytes)?;
    if bytes.len() != 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ipc.secret is corrupted (expected 32 bytes); stop all gateways and remove only this secret to regenerate",
        ));
    }
    Ok(bytes.to_vec())
}

/// Caller holds the daemon's exclusive state lock. Never replaces existing keys.
pub(crate) fn ensure(path: &Path) -> io::Result<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(_) => return read(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("secret path has no parent"))?;
    crate::private_directory(parent)?;
    let temporary = parent.join("ipc.secret.tmp");
    if fs::symlink_metadata(&temporary).is_ok() {
        fs::remove_file(&temporary)?;
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    crate::private_file(&temporary)?;
    let bytes = zeroize::Zeroizing::new(crate::handshake::nonce().map_err(io::Error::other)?);
    file.write_all(bytes.as_ref())?;
    file.sync_all()?;
    drop(file);
    let result = fs::hard_link(&temporary, path);
    fs::remove_file(&temporary)?;
    match result {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    read(path)
}
