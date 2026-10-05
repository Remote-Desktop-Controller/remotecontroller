use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_local-daemon"))
}
fn command(workspace: &Path, profile: &Path) -> Command {
    let mut command = Command::new(binary());
    command
        .args(["--workspace"])
        .arg(workspace)
        .arg("--profile")
        .arg(profile);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
#[test]
fn state_is_rejected_before_creating_an_in_workspace_directory() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let state = workspace.join("private-state");
    let output = command(&workspace, &directory.path().join("profile"))
        .arg("--state-dir")
        .arg(&state)
        .arg("init")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !state.exists(),
        "Invalid state directory was created before rejection"
    );
}
#[test]
fn profile_is_rejected_before_creating_an_in_workspace_directory() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let profile = workspace.join("private-profile");
    let output = command(&workspace, &profile).arg("init").output().unwrap();
    assert!(!output.status.success());
    assert!(
        !profile.exists(),
        "Invalid profile directory was created before rejection"
    );
}
#[test]
fn missing_explicit_configuration_reports_the_operator_path() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let output = command(&workspace, &directory.path().join("profile"))
        .arg("--config")
        .arg(directory.path().join("missing.json"))
        .arg("doctor")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--config"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
