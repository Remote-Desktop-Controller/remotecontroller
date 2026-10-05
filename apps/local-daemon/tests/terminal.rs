use runtime_domain::*;
use runtime_infrastructure::{
    config::RuntimeConfig, kernel::Kernel, processes::ProgramRule, terminal::TerminalConfig,
};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::{Value, json};
use std::sync::Arc;

async fn call(k: &Arc<Kernel>, tool: &str, input: Value, meta: Metadata) -> Value {
    k.handle(
        RequestEnvelope {
            meta,
            payload: Request::Execute {
                tool: tool.into(),
                input,
            },
        },
        Arc::new(NoProgress),
    )
    .await
    .payload
    .unwrap()
}
fn meta(k: &Kernel) -> Metadata {
    Metadata::new(Some(k.workspace.id.as_uuid()), 10000)
}
async fn await_file(path: &std::path::Path, k: &Arc<Kernel>, id: &str) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let output = call(
        k,
        "terminal.read",
        json!({"terminal_id":id,"offset":0,"max_bytes":4096}),
        meta(k),
    )
    .await;
    panic!(
        "terminal did not create {}; captured={output}",
        path.display()
    );
}
#[tokio::test]
async fn real_pty_write_retry_resize_close_tree_and_durable_output() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let program = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32")
        .join("cmd.exe");
    #[cfg(unix)]
    let program = std::path::PathBuf::from("/bin/sh");
    let config = RuntimeConfig {
        guardian_path: Some(std::path::PathBuf::from(env!("CARGO_BIN_EXE_local-daemon"))),
        terminal: TerminalConfig {
            enabled: true,
            fingerprint: Some(ProgramRule::fingerprint(&program).unwrap()),
            program: Some(program),
            args: if cfg!(windows) {
                vec!["/D".into(), "/Q".into(), "/K".into()]
            } else {
                vec![]
            },
            idle_timeout_seconds: 30,
            ..TerminalConfig::default()
        },
        ..RuntimeConfig::default()
    };
    let policy = ExecutionPolicy::new([
        Capability::ReadWorkspace,
        Capability::AllowInteractiveTerminal,
        Capability::OutsideWorkspaceAccess,
        Capability::NetworkAccess,
    ]);
    let k = Kernel::open(root.path(), state.path(), config.clone(), policy.clone())
        .await
        .unwrap();
    let open = call(&k, "terminal.open", json!({}), meta(&k)).await;
    assert_eq!(open["status"], "Succeeded", "{open}");
    let id = open["result"]["terminal_id"].as_str().unwrap().to_owned();
    let write_meta = meta(&k);
    let line = "echo unique-line>> terminal-proof\r\n";
    let written = call(
        &k,
        "terminal.write",
        json!({"terminal_id":id,"text":line}),
        write_meta.clone(),
    )
    .await;
    assert_eq!(written["status"], "Succeeded", "{written}");
    await_file(&root.path().join("terminal-proof"), &k, &id).await;
    assert_eq!(
        call(
            &k,
            "terminal.write",
            json!({"terminal_id":id,"text":line}),
            write_meta
        )
        .await,
        written
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("terminal-proof"))
            .unwrap()
            .matches("unique-line")
            .count(),
        1
    );
    let resized = call(
        &k,
        "terminal.resize",
        json!({"terminal_id":id,"rows":40,"cols":100}),
        meta(&k),
    )
    .await;
    assert_eq!(resized["status"], "Succeeded", "{resized}");
    let quota = call(
        &k,
        "terminal.write",
        json!({"terminal_id":id,"text":"x".repeat(4097)}),
        meta(&k),
    )
    .await;
    assert_eq!(quota["status"], "Failed");
    #[cfg(windows)]
    let background = "start /b \"\" cmd /d /q /c \"echo ready>child-started & ping -n 4 127.0.0.1>nul & echo survived>child-survived\"\r\n";
    #[cfg(unix)]
    let background =
        "/bin/sh -c 'echo ready>child-started; /bin/sleep 2; echo survived>child-survived' &\n";
    assert_eq!(
        call(
            &k,
            "terminal.write",
            json!({"terminal_id":id,"text":background}),
            meta(&k)
        )
        .await["status"],
        "Succeeded"
    );
    await_file(&root.path().join("child-started"), &k, &id).await;
    let closed = call(&k, "terminal.close", json!({"terminal_id":id}), meta(&k)).await;
    assert_eq!(closed["status"], "Succeeded", "{closed}");
    k.services.terminal.as_ref().unwrap().shutdown().await;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    assert!(
        !root.path().join("child-survived").exists(),
        "descendant outlived terminal close"
    );
    let saved = call(
        &k,
        "terminal.read",
        json!({"terminal_id":id,"max_bytes":131072}),
        meta(&k),
    )
    .await;
    assert_eq!(saved["status"], "Succeeded");
    assert_eq!(saved["result"]["encoding"], "hex");
    assert_eq!(saved["result"]["status"], "Exited", "{saved}");
    assert_eq!(saved["result"]["capture_incomplete"], false, "{saved}");
    assert!(
        saved["result"]["text"]
            .as_str()
            .unwrap()
            .contains("unique-line")
    );
    drop(k);
    let k = Kernel::open(root.path(), state.path(), config, policy)
        .await
        .unwrap();
    let replay = call(
        &k,
        "terminal.read",
        json!({"terminal_id":id,"max_bytes":131072}),
        meta(&k),
    )
    .await;
    assert_eq!(saved["result"]["data"], replay["result"]["data"]);
}
#[tokio::test]
async fn terminal_requires_local_opt_in_and_capabilities() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let denied = k
        .handle(
            RequestEnvelope {
                meta: meta(&k),
                payload: Request::Execute {
                    tool: "terminal.open".into(),
                    input: json!({}),
                },
            },
            Arc::new(NoProgress),
        )
        .await
        .payload
        .unwrap_err();
    assert_eq!(denied.code, ErrorCode::PolicyDenied);
    let state2 = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state2.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::AllowInteractiveTerminal,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    let disabled = call(&k, "terminal.open", json!({}), meta(&k)).await;
    assert_eq!(disabled["status"], "Failed");
    assert!(disabled["error"].as_str().unwrap().contains("disabled"));
}

#[test]
fn local_terminal_approval_pins_shell_and_preserves_process_rules() {
    let root = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let program =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
    #[cfg(unix)]
    let program = std::path::PathBuf::from("/bin/sh");
    let config = profile.path().join("runtime.json");
    let original = RuntimeConfig {
        process_spool_bytes: 10000,
        ..RuntimeConfig::default()
    };
    std::fs::write(&config, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    let execute = |approve: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_local-daemon"));
        command
            .arg("--workspace")
            .arg(root.path())
            .arg("--profile")
            .arg(profile.path())
            .arg("--config")
            .arg(&config)
            .arg("terminal")
            .arg(if approve { "approve" } else { "revoke" });
        if approve {
            command
                .arg("--program")
                .arg(&program)
                .arg("--trust-unconfined")
                .arg("--")
                .arg("-fixture-arg");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    execute(true);
    let approved: RuntimeConfig = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert!(approved.terminal.enabled);
    assert_eq!(
        approved.terminal.program,
        Some(program.canonicalize().unwrap())
    );
    assert_eq!(
        approved.terminal.fingerprint,
        Some(ProgramRule::fingerprint(&program).unwrap())
    );
    assert_eq!(approved.terminal.args, ["-fixture-arg"]);
    assert!(approved.allowed_programs.is_empty());
    assert_eq!(approved.process_spool_bytes, original.process_spool_bytes);
    execute(false);
    let revoked: RuntimeConfig = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert!(!revoked.terminal.enabled);
}
