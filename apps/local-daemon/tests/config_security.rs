//! Policy configuration must remain outside MCP-writable workspace paths.
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn init(workspace: &Path, profile: &Path, config: Option<&Path>) -> Output {
    let binary = std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join(format!("local-daemon{}", std::env::consts::EXE_SUFFIX)))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_local-daemon")));
    let mut cmd = Command::new(binary);
    cmd.arg("init")
        .arg("--workspace")
        .arg(workspace)
        .arg("--profile")
        .arg(profile);
    if let Some(config) = config {
        cmd.arg("--config").arg(config);
    }
    cmd.output().unwrap()
}

fn rejected(output: Output) {
    assert!(
        !output.status.success(),
        "workspace configuration was accepted"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("configuration must be outside workspace"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn refuses_missing_workspace_config_before_writing() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let config = workspace.join("runtime.json");
    rejected(init(&workspace, &t.path().join("profile"), Some(&config)));
    assert!(!config.exists());
}

#[test]
fn refuses_existing_workspace_policy_config() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let config = workspace.join("runtime.json");
    std::fs::write(&config, b"existing policy").unwrap();
    rejected(init(&workspace, &t.path().join("profile"), Some(&config)));
    assert_eq!(std::fs::read(&config).unwrap(), b"existing policy");
}

#[test]
fn default_config_is_initialized_in_private_state() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let profile = t.path().join("profile");
    let output = init(&workspace, &profile, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(config.is_file());
    assert!(
        config
            .canonicalize()
            .unwrap()
            .starts_with(profile.canonicalize().unwrap())
    );
    assert!(!workspace.join("runtime.json").exists());
}

#[cfg(unix)]
#[test]
fn refuses_config_symlink_outside_workspace_pointing_inside() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let policy = workspace.join("runtime.json");
    std::fs::write(&policy, b"existing policy").unwrap();
    let link = t.path().join("config-link.json");
    std::os::unix::fs::symlink(&policy, &link).unwrap();
    rejected(init(&workspace, &t.path().join("profile"), Some(&link)));
    assert_eq!(std::fs::read(&policy).unwrap(), b"existing policy");
}

#[cfg(unix)]
#[test]
fn refuses_missing_config_under_directory_symlink_into_workspace() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let link = t.path().join("workspace-link");
    std::os::unix::fs::symlink(&workspace, &link).unwrap();
    let config = link.join("runtime.json");
    rejected(init(&workspace, &t.path().join("profile"), Some(&config)));
    assert!(!workspace.join("runtime.json").exists());
}

#[cfg(unix)]
#[test]
fn refuses_lexical_workspace_config_even_when_symlink_points_outside() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let link = workspace.join("outside-link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    rejected(init(
        &workspace,
        &t.path().join("profile"),
        Some(&link.join("runtime.json")),
    ));
    assert!(!outside.join("runtime.json").exists());
}

#[cfg(unix)]
#[test]
fn refuses_config_under_real_workspace_when_workspace_uses_alias() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let alias = t.path().join("workspace-alias");
    std::os::unix::fs::symlink(&workspace, &alias).unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let link = workspace.join("outside-link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    rejected(init(
        &alias,
        &t.path().join("profile"),
        Some(&link.join("runtime.json")),
    ));
    assert!(!outside.join("runtime.json").exists());
}

#[cfg(unix)]
#[test]
fn refuses_config_crossing_workspace_through_distinct_parent_alias() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace_alias = t.path().join("workspace-alias");
    std::os::unix::fs::symlink(&workspace, &workspace_alias).unwrap();
    let parent_alias = t.path().join("parent-alias");
    std::os::unix::fs::symlink(t.path(), &parent_alias).unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let existing_config = outside.join("existing.json");
    std::fs::write(&existing_config, b"existing external configuration").unwrap();
    let outside_link = workspace.join("outside-link");
    std::os::unix::fs::symlink(&outside, &outside_link).unwrap();
    let config = parent_alias
        .join("workspace")
        .join("outside-link")
        .join("runtime.json");
    rejected(init(
        &workspace_alias,
        &t.path().join("profile"),
        Some(&config),
    ));
    assert!(!outside.join("runtime.json").exists());
    assert_eq!(
        std::fs::read(existing_config).unwrap(),
        b"existing external configuration"
    );
}
