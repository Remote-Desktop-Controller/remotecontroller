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
