use runtime_transport::{ensure_secret, private_file, read_secret};

#[test]
fn secret_creation_is_atomic_private_and_preserves_invalid_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ipc.secret");
    std::fs::write(directory.path().join("ipc.secret.tmp"), b"incomplete").unwrap();
    let first = ensure_secret(&path).unwrap();
    assert_eq!(first.len(), 32);
    assert_eq!(read_secret(&path).unwrap(), first);
    assert_eq!(ensure_secret(&path).unwrap(), first);
    assert!(!directory.path().join("ipc.secret.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::write(&path, b"invalid").unwrap();
    assert!(ensure_secret(&path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"invalid");
}

#[test]
fn wrong_secret_length_reports_recovery_action() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ipc.secret");
    for length in [31, 33] {
        std::fs::write(&path, vec![7; length]).unwrap();
        private_file(&path).unwrap();
        let error = read_secret(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("corrupted"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn secret_read_rejects_public_permissions_and_symlinks_at_open() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ipc.secret");
    std::fs::write(&path, [7; 32]).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        read_secret(&path).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    private_file(&path).unwrap();
    let link = directory.path().join("link");
    symlink(path, &link).unwrap();
    assert!(read_secret(&link).is_err());
}
