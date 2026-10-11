use std::process::Command;

#[test]
fn unpaired_status_returns_success_and_json_instruction() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("status")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        output.status.success(),
        "status must succeed when unpaired, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout_str)
        .unwrap_or_else(|e| panic!("Output should be valid JSON: {e}, got: {stdout_str}"));

    assert_eq!(parsed["status"], "unpaired");
    assert_eq!(parsed["paired"], false);
    assert!(
        parsed["instructions"]
            .as_str()
            .unwrap_or_default()
            .contains("device-agent pair"),
        "Instructions must mention device-agent pair, got: {stdout_str}"
    );
    assert!(
        !stdout_str.contains("os error 2"),
        "Must not contain os error 2"
    );
}

#[test]
fn unpaired_run_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("run")
        .output()
        .expect("failed to execute device-agent");

    assert!(!output.status.success(), "run should fail when unpaired");
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout_str}\n{stderr_str}");

    assert!(
        combined.contains("Dispositivo não pareado. Execute o comando de pareamento antes de iniciar o agente: device-agent pair --server <URL> --workspace <PASTA>"),
        "Must contain friendly unpaired guidance, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_stop_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("stop")
        .output()
        .expect("failed to execute device-agent");

    assert!(!output.status.success(), "stop should fail when unpaired");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Dispositivo não pareado"),
        "Must mention device is unpaired, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_approve_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("approve")
        .arg("--program")
        .arg("fake.exe")
        .arg("--trust-unconfined")
        .output()
        .expect("failed to execute device-agent");

    assert!(!output.status.success(), "approve should fail when unpaired");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Dispositivo não pareado"),
        "Must mention device is unpaired, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_terminal_approve_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("terminal-approve")
        .arg("--program")
        .arg("powershell.exe")
        .arg("--trust-unconfined")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        !output.status.success(),
        "terminal-approve should fail when unpaired"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Dispositivo não pareado"),
        "Must mention device is unpaired, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_autostart_enable_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("autostart")
        .arg("enable")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        !output.status.success(),
        "autostart enable should fail when unpaired"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Dispositivo não pareado"),
        "Must mention device is unpaired, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_default_profile_in_isolated_environment_reports_unpaired() {
    let home = tempfile::tempdir().unwrap();
    let localappdata = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", localappdata.path())
        .arg("status")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        output.status.success(),
        "status with default profile in isolated env must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout_str)
        .unwrap_or_else(|e| panic!("Output should be valid JSON: {e}, got: {stdout_str}"));

    assert_eq!(parsed["status"], "unpaired");
    assert_eq!(parsed["paired"], false);
    let expected_agent_json = home
        .path()
        .join(".remotecontroller")
        .join("agent")
        .join("agent.json");
    assert_eq!(
        parsed["profile_path"],
        expected_agent_json.to_string_lossy().as_ref()
    );
}

#[test]
fn unpaired_autostart_disable_fails_with_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("agent");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&profile_dir)
        .arg("autostart")
        .arg("disable")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        !output.status.success(),
        "autostart disable should fail when unpaired"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Dispositivo não pareado"),
        "Must mention device is unpaired, got: {combined}"
    );
    assert!(
        !combined.contains("os error 2"),
        "Must not expose raw OS error 2, got: {combined}"
    );
}

#[test]
fn unpaired_status_with_profile_pointing_directly_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let agent_json = dir.path().join("agent.json");

    let output = Command::new(env!("CARGO_BIN_EXE_device-agent"))
        .arg("--profile")
        .arg(&agent_json)
        .arg("status")
        .output()
        .expect("failed to execute device-agent");

    assert!(
        output.status.success(),
        "status must succeed when unpaired, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout_str)
        .unwrap_or_else(|e| panic!("Output should be valid JSON: {e}, got: {stdout_str}"));

    assert_eq!(parsed["status"], "unpaired");
    assert_eq!(parsed["paired"], false);
    assert_eq!(parsed["profile_path"], agent_json.to_string_lossy().as_ref());
}
