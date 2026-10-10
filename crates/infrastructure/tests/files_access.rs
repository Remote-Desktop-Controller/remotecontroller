use runtime_infrastructure::files::{AccessScope, FileLimits, WorkspaceFiles};
use runtime_ports::{FileSystemPort, PortError};
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn full_computer_access_relative_and_absolute() {
    let user_home = tempdir().unwrap();
    let other_drive = tempdir().unwrap();

    let downloads = user_home.path().join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    fs::write(downloads.join("report.txt"), b"relatorio 2026").unwrap();

    let project = other_drive.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("main.rs"), b"fn main() {}").unwrap();

    let mut limits = FileLimits::default();
    limits.scope = AccessScope::FullComputer;
    let files = WorkspaceFiles::open(user_home.path(), limits).unwrap();

    // 1. Relative read starting at user home
    let content = files.read("Downloads/report.txt").await.unwrap();
    assert_eq!(content, b"relatorio 2026");

    // 2. Absolute read
    let abs_path = project.join("main.rs").to_string_lossy().to_string();
    let content_abs = files.read(&abs_path).await.unwrap();
    assert_eq!(content_abs, b"fn main() {}");
}

#[tokio::test]
async fn full_computer_enclave_blocks_credentials_and_state() {
    let user_home = tempdir().unwrap();

    // Sensitive folders
    let ssh = user_home.path().join(".ssh");
    fs::create_dir_all(&ssh).unwrap();
    fs::write(ssh.join("id_rsa"), b"PRIVATE KEY").unwrap();

    let app_data = user_home.path().join("AppData/Local/RemoteDesktopController");
    fs::create_dir_all(&app_data).unwrap();
    fs::write(app_data.join("agent.json"), b"SECRET TOKEN").unwrap();

    let mut limits = FileLimits::default();
    limits.scope = AccessScope::FullComputer;
    limits.enclaves = vec![app_data.clone()];
    let files = WorkspaceFiles::open(user_home.path(), limits).unwrap();

    // Attempt to read .ssh/id_rsa relative
    let err_ssh = files.read(".ssh/id_rsa").await.unwrap_err();
    assert!(matches!(err_ssh, PortError::Policy(_)));

    // Attempt to read agent.json absolute
    let abs_agent = app_data.join("agent.json").to_string_lossy().to_string();
    let err_agent = files.read(&abs_agent).await.unwrap_err();
    assert!(matches!(err_agent, PortError::Policy(_)));
}

#[tokio::test]
async fn restricted_mode_still_blocks_outside_files() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), b"outside").unwrap();

    let mut limits = FileLimits::default();
    limits.scope = AccessScope::Restricted;
    let files = WorkspaceFiles::open(workspace.path(), limits).unwrap();

    let outside_path = outside.path().join("secret.txt").to_string_lossy().to_string();
    let err = files.read(&outside_path).await.unwrap_err();
    assert!(matches!(err, PortError::Policy(_)));
}

#[tokio::test]
async fn full_computer_scan_discovers_files_and_skips_enclaves() {
    let user_home = tempdir().unwrap();
    let docs = user_home.path().join("Documents");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("contract.pdf"), b"pdf content").unwrap();

    let ssh = user_home.path().join(".ssh");
    fs::create_dir_all(&ssh).unwrap();
    fs::write(ssh.join("id_ed25519"), b"private").unwrap();

    let mut limits = FileLimits::default();
    limits.scope = AccessScope::FullComputer;
    let files = WorkspaceFiles::open(user_home.path(), limits).unwrap();

    let token = tokio_util::sync::CancellationToken::new();
    let scanned = files.scan("Documents", 100, token).await.unwrap();
    assert_eq!(scanned.len(), 1);
    assert!(scanned[0].ends_with("Documents/contract.pdf"));

    let token2 = tokio_util::sync::CancellationToken::new();
    let all_scanned = files.scan("", 100, token2).await.unwrap();
    // Verify .ssh is omitted from scan
    assert!(!all_scanned.iter().any(|p| p.contains(".ssh")));
}

#[tokio::test]
async fn full_computer_root_deletion_rejected() {
    let user_home = tempdir().unwrap();
    let mut limits = FileLimits::default();
    limits.scope = AccessScope::FullComputer;
    let files = WorkspaceFiles::open(user_home.path(), limits).unwrap();

    // Trying to delete root or "."
    let snap = runtime_domain::FileSnapshot::new(".", None);
    let err = files.restore(vec![snap]).await.unwrap_err();
    assert!(matches!(err, PortError::Policy(_)));
}

