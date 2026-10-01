use runtime_infrastructure::files::{FileLimits, WorkspaceFiles};
use runtime_ports::FileSystemPort;
#[tokio::test]
async fn oversized_ignore_policy_is_denied() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".gitignore"), "\n".repeat(65537)).unwrap();
    std::fs::write(root.path().join("secret"), "secret").unwrap();
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    assert!(files.read("secret").await.is_err());
}
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn traversal_ignore_and_atomic_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "hidden").unwrap();
    let fs = WorkspaceFiles::open(dir.path(), FileLimits::default()).unwrap();
    assert!(fs.read("../escape").await.is_err());
    assert!(fs.read("secret.txt").await.is_err());
    assert!(fs.write_atomic("target/x", vec![]).await.is_err());
    fs.write_atomic("safe.txt", b"first".to_vec())
        .await
        .unwrap();
    fs.write_atomic("safe.txt", b"second".to_vec())
        .await
        .unwrap();
    assert_eq!(fs.read("safe.txt").await.unwrap(), b"second");
    assert_eq!(
        fs.scan(".", 100, CancellationToken::new())
            .await
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
async fn directory_link_escape_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "private").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(dir.path().join("escape"))
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(status.status.success());
    }
    let fs = WorkspaceFiles::open(dir.path(), FileLimits::default()).unwrap();
    assert!(fs.read("escape/secret").await.is_err());
    assert!(fs.write_atomic("escape/new", vec![1]).await.is_err());
    assert!(!outside.path().join("new").exists());
}

#[tokio::test]
async fn atomic_write_preserves_readonly_and_timestamps() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("original.bin");
    std::fs::write(&path, [0, 255, 1, 128]).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    file.set_times(
        std::fs::FileTimes::new()
            .set_accessed(timestamp)
            .set_modified(timestamp),
    )
    .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions).unwrap();
    drop(file);
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let outcome = files.write_atomic("original.bin", vec![3, 255, 0]).await;
    // Windows may reject replacement of a readonly target. Either preserve it
    // without mutation, or restore all supported properties on successful writes.
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.permissions().readonly());
    assert_eq!(metadata.modified().unwrap(), timestamp);
    if outcome.is_ok() {
        assert_eq!(std::fs::read(&path).unwrap(), [3, 255, 0]);
    } else {
        assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 1, 128]);
    }
    #[cfg(windows)]
    {
        let mut permissions = metadata.permissions();
        // Windows-only cleanup of this readonly temporary fixture.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(metadata.permissions().mode() | 0o200),
        )
        .unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_write_preserves_executable_mode() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("program");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    files
        .write_atomic("program", b"new".to_vec())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777,
        0o751
    );
}

#[tokio::test]
async fn atomic_write_preserves_modified_time_root_and_nested() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    for name in ["root.bin", "nested/file.bin"] {
        let path = root.path().join(name);
        std::fs::write(&path, [0, 255]).unwrap();
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        files.write_atomic(name, vec![255, 0, 1]).await.unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            timestamp
        );
    }
}

#[tokio::test]
async fn legacy_rollback_does_not_overwrite_external_binary_edit() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.bin"), [255, 0, 7]).unwrap();
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let original = runtime_domain::FileSnapshot::new("file.bin", Some(vec![0, 128, 1]));
    assert!(files.restore(vec![original]).await.is_err());
    assert_eq!(
        std::fs::read(root.path().join("file.bin")).unwrap(),
        [255, 0, 7]
    );
}
