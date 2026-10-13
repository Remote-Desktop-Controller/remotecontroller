//! `serve` must accept every permission flag the commercial device-agent forwards,
//! and must expose the telemetry pipe so `rdc-monitor` can attach.
#![cfg(windows)]
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn binary() -> PathBuf {
    std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join(format!("local-daemon{}", std::env::consts::EXE_SUFFIX)))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_local-daemon")))
}

#[test]
fn serve_accepts_agent_permission_flags_and_exposes_telemetry_pipe() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let profile = t.path().join("profile");
    let endpoint = format!(r"\\.\pipe\rdc-test-serve-{}", uuid::Uuid::new_v4().simple());

    let mut child = Command::new(binary())
        .arg("serve")
        .arg("--workspace")
        .arg(&workspace)
        .arg("--profile")
        .arg(&profile)
        .arg("--endpoint")
        .arg(&endpoint)
        .args([
            "--full-computer",
            "--allow-write",
            "--allow-process",
            "--allow-terminal",
            "--allow-process-outside-workspace",
            "--allow-process-network",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // A clap rejection exits immediately; a healthy daemon keeps running.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut early_exit = None;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            early_exit = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(status) = early_exit {
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!("serve exited early ({status}): {stderr}");
    }

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let connected = runtime.block_on(async {
        for _ in 0..50 {
            if runtime_infrastructure::observer::server::connect_telemetry_client(
                runtime_infrastructure::observer::DEFAULT_TELEMETRY_PIPE,
            )
            .await
            .is_ok()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    });
    let _ = child.kill();
    let _ = child.wait();
    assert!(connected, "telemetry pipe never became connectable");
}
