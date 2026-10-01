//! Real native install/bootstrap and MCP wire exchange in an isolated user profile.
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
fn args(cmd: &mut Command, workspace: &Path, profile: &Path) {
    cmd.arg("--workspace")
        .arg(workspace)
        .arg("--profile")
        .arg(profile);
}
fn binary(name: &str) -> PathBuf {
    let directory = std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_BIN_EXE_local-daemon"))
                .parent()
                .unwrap()
                .to_path_buf()
        });
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}
#[test]
fn installed_connect_bootstraps_real_daemon_and_mcp() {
    let t = tempfile::tempdir().unwrap();
    let workspace = t.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let profile = t.path().join("profile");
    let source = t.path().join("bundle");
    std::fs::create_dir(&source).unwrap();
    let mut files = serde_json::Map::new();
    for name in ["local-daemon", "mcp-gateway"] {
        let p = binary(name);
        assert!(
            p.exists(),
            "build both binaries before native lifecycle test: {}",
            p.display()
        );
        let n = p.file_name().unwrap();
        std::fs::copy(&p, source.join(n)).unwrap();
        let h = format!("{:x}", Sha256::digest(std::fs::read(&p).unwrap()));
        files.insert(n.to_string_lossy().into_owned(), h.into());
    }
    std::fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({"version":"0.2.0-test","files":files})).unwrap(),
    )
    .unwrap();
    let mut install = Command::new(binary("local-daemon"));
    install.arg("install").arg("--source").arg(&source);
    args(&mut install, &workspace, &profile);
    let output = install.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let release = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let exe = release.join(format!("local-daemon{}", std::env::consts::EXE_SUFFIX));
    let mut connect = Command::new(&exe);
    connect.arg("connect");
    args(&mut connect, &workspace, &profile);
    let mut child = connect
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    writeln!(input,"{}",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"native-installer-test","version":"1"}}})).unwrap();
    input.flush().unwrap();
    let initialized: serde_json::Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap()).unwrap();
    assert_eq!(initialized["id"], 1);
    assert!(initialized["result"]["serverInfo"].is_object());
    writeln!(
        input,
        "{}",
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    input.flush().unwrap();
    let tools: serde_json::Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()).unwrap();
    assert_eq!(tools["id"], 2);
    assert!(!tools["result"]["tools"].as_array().unwrap().is_empty());
    drop(input);
    let _ = child.wait();
    let mut stop = Command::new(&exe);
    stop.arg("stop");
    args(&mut stop, &workspace, &profile);
    assert!(stop.status().unwrap().success());
    let mut updated_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("manifest.json")).unwrap()).unwrap();
    updated_manifest["version"] = "0.2.1-test".into();
    std::fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&updated_manifest).unwrap(),
    )
    .unwrap();
    let mut update = Command::new(&exe);
    update.arg("update").arg("--source").arg(&source);
    args(&mut update, &workspace, &profile);
    let updated = update.output().unwrap();
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let updated_release = PathBuf::from(String::from_utf8(updated.stdout).unwrap().trim());
    assert_ne!(updated_release, release);
    assert!(exe.exists(), "update must preserve previous release");
    let updated_exe = updated_release.join(format!("local-daemon{}", std::env::consts::EXE_SUFFIX));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        struct Guard(std::process::Child);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let state = std::fs::read_dir(profile.join("states"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut serve = Command::new(&updated_exe);
        serve.arg("serve");
        args(&mut serve, &workspace, &profile);
        let mut daemon = Guard(
            serve
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let mut doctor = Command::new(&updated_exe);
            doctor.arg("doctor");
            args(&mut doctor, &workspace, &profile);
            if doctor.output().unwrap().status.success() {
                break;
            }
            assert!(daemon.0.try_wait().unwrap().is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(100));
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(state.canonicalize().unwrap().to_string_lossy().as_bytes())
        );
        let mut gateway = Guard(
            Command::new(updated_release.join("mcp-gateway.exe"))
                .arg("--endpoint")
                .arg(format!(r"\\.\pipe\local-runtime-{}", &hash[..16]))
                .arg("--token-file")
                .arg(state.join("ipc.secret"))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap(),
        );
        let stderr = gateway.0.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        loop {
            let line = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            if line.contains("MCP gateway connected") {
                break;
            }
        }
        let mut stop = Command::new(&updated_exe);
        stop.arg("stop");
        args(&mut stop, &workspace, &profile);
        assert!(stop.status().unwrap().success());
        assert!(daemon.0.wait().unwrap().success());
        assert!(
            gateway.0.try_wait().unwrap().is_none(),
            "real gateway fixture must be active"
        );
        let mut refused = Command::new(binary("local-daemon"));
        refused.arg("uninstall");
        args(&mut refused, &workspace, &profile);
        let output = refused.output().unwrap();
        assert!(
            !output.status.success(),
            "uninstall must refuse busy gateway"
        );
        assert!(profile.join("active.json").is_file());
        for name in ["local-daemon.exe", "mcp-gateway.exe", "manifest.json"] {
            assert!(
                updated_release.join(name).is_file(),
                "busy release must remain complete"
            );
        }
        gateway.0.kill().unwrap();
        gateway.0.wait().unwrap();
    }
    // State persists after uninstall; running programs have stopped first.
    std::thread::sleep(Duration::from_millis(300));
    let mut uninstall = Command::new(&updated_exe);
    uninstall.arg("uninstall");
    args(&mut uninstall, &workspace, &profile);
    assert!(uninstall.status().unwrap().success());
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while profile.join("active.json").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "installed executable did not finish uninstall"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    #[cfg(windows)]
    {
        loop {
            let status: serde_json::Value = serde_json::from_slice(
                &std::fs::read(profile.join("uninstall-status.json")).unwrap(),
            )
            .unwrap();
            if status["success"].as_bool() == Some(true) {
                break;
            }
            assert_ne!(status["success"], false, "{status}");
            assert!(std::time::Instant::now() < deadline, "{status}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    assert!(profile.join("states").is_dir());
}
