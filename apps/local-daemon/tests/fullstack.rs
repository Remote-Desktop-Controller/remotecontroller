//! Real OS processes and MCP stdio, no fake executor or storage.
use runtime_protocol::*;
use runtime_transport::{Client, read_secret};
use serde_json::{Value, json};
use std::sync::Arc;
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

fn binary(name: &str) -> std::path::PathBuf {
    let directory = std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_BIN_EXE_local-daemon"))
                .parent()
                .unwrap()
                .to_path_buf()
        });
    directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}
async fn start(root: &Path, state: &Path, endpoint: &str) -> Child {
    let mut command = Command::new(binary("local-daemon"));
    command
        .args(["--workspace"])
        .arg(root)
        .arg("--state-dir")
        .arg(state)
        .args(["--endpoint", endpoint, "--allow-write"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().unwrap();
    let diagnostics = Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new()));
    let collected = diagnostics.clone();
    let stderr = child.stderr.take().unwrap();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut output = collected.lock().await;
            output.push_back(line);
            while output.len() > 64 {
                output.pop_front();
            }
        }
    });
    let started = std::time::Instant::now();
    let mut last_error = String::from("waiting for secret file");
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "daemon exited: {status}; stderr={:?}; last IPC error={last_error}",
                diagnostics.lock().await
            );
        }
        if let Ok(secret) = read_secret(&state.join("ipc.secret")) {
            let client = Client::new(endpoint.into(), secret);
            match client
                .request(
                    &RequestEnvelope {
                        meta: Metadata::new(None, 1000),
                        payload: Request::Heartbeat,
                    },
                    |_| {},
                )
                .await
            {
                Ok(response) if response.payload.is_ok() => break,
                Ok(response) => last_error = format!("{:?}", response.payload),
                Err(error) => last_error = error.to_string(),
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "daemon failed to become ready; stderr={:?}; last IPC error={last_error}",
            diagnostics.lock().await
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child
}
async fn message(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    message: Value,
) -> Value {
    input
        .write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
    input.flush().await.unwrap();
    let line = tokio::time::timeout(Duration::from_secs(10), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(&line).expect("MCP stdout must contain only JSON-RPC protocol")
}
#[tokio::test]
async fn mcp_to_daemon_abrupt_restart_reconnect_idempotency_and_stdout() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\rdc-fullstack-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = state
        .path()
        .join("fullstack.sock")
        .to_string_lossy()
        .to_string();
    let mut daemon = start(root.path(), state.path(), &endpoint).await;
    let gateway_path = binary("mcp-gateway");
    assert!(
        gateway_path.exists(),
        "run cargo build --workspace before fullstack tests"
    );
    let mut command = Command::new(gateway_path);
    command
        .args(["--endpoint", &endpoint, "--token-file"])
        .arg(state.path().join("ipc.secret"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut gateway = command.spawn().unwrap();
    let mut input = gateway.stdin.take().unwrap();
    let mut output = BufReader::new(gateway.stdout.take().unwrap()).lines();
    let initialize=message(&mut input,&mut output,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"integration","version":"1"}}})).await;
    assert!(initialize.get("result").is_some(), "{initialize}");
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    let catalog = message(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert!(
        catalog["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "filesystem.write")
    );
    let operation = uuid::Uuid::new_v4();
    let params = json!({"name":"filesystem.write","arguments":{"path":"result.txt","content":"from MCP"},"_meta":{"runtimeOperationId":operation.to_string()}});
    let first = message(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":params}),
    )
    .await;
    assert_eq!(
        first["result"]["structuredContent"]["status"], "Succeeded",
        "{first}"
    );
    daemon.kill().await.unwrap();
    daemon.wait().await.unwrap();
    let mut restarted = start(root.path(), state.path(), &endpoint).await;
    let second = message(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":params}),
    )
    .await;
    assert_eq!(
        first["result"]["structuredContent"]["operation_id"],
        second["result"]["structuredContent"]["operation_id"],
        "{second}"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("result.txt")).unwrap(),
        "from MCP"
    );
    let causal=message(&mut input,&mut output,json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"context.causal","arguments":{"operation_id":operation.to_string()}}})).await;
    assert!(
        causal["result"]["structuredContent"]["result"]["context"]
            .as_str()
            .is_some_and(|context| context.contains("filesystem.write")),
        "unexpected causal response: {causal}"
    );
    drop(input);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), gateway.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    // Any remaining stdout must also be protocol. Logs use stderr exclusively.
    while let Some(line) = output.next_line().await.unwrap() {
        serde_json::from_str::<Value>(&line).unwrap();
    }
    restarted.kill().await.unwrap();
    restarted.wait().await.unwrap();
}
#[tokio::test]
async fn abrupt_crash_during_batch_restores_journal_before_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    for i in 0..1000 {
        std::fs::write(root.path().join(format!("f{i}.txt")), "original").unwrap();
    }
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\rdc-crash-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = state
        .path()
        .join("crash.sock")
        .to_string_lossy()
        .to_string();
    let mut daemon = start(root.path(), state.path(), &endpoint).await;
    let client = Client::new(
        endpoint.clone(),
        read_secret(&state.path().join("ipc.secret")).unwrap(),
    );
    let catalog = client
        .request(
            &RequestEnvelope {
                meta: Metadata::new(None, 5000),
                payload: Request::Catalog,
            },
            |_| {},
        )
        .await
        .unwrap()
        .payload
        .unwrap();
    let workspace = catalog["workspace_id"].as_str().unwrap().parse().unwrap();
    let edits: Vec<_> = (0..1000)
        .map(|i| json!({"path":format!("f{i}.txt"),"content":"changed"}))
        .collect();
    let request = RequestEnvelope {
        meta: Metadata::new(Some(workspace), 30_000),
        payload: Request::Execute {
            tool: "filesystem.batch_edit".into(),
            input: json!({"edits":edits}),
        },
    };
    let retry = request.clone();
    let worker_client = client.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let worker = tokio::spawn(async move {
        worker_client
            .request(&request, |e| {
                let _ = tx.try_send(e.completed);
            })
            .await
    });
    loop {
        let done = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        if done >= 64 {
            break;
        }
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("f0.txt")).unwrap(),
        "changed"
    );
    daemon.kill().await.unwrap();
    daemon.wait().await.unwrap();
    assert!(worker.await.unwrap().is_err());
    let mut resumed = start(root.path(), state.path(), &endpoint).await;
    let failed = client
        .request(&retry, |_| {})
        .await
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(failed["status"], "Failed");
    for i in 0..1000 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("f{i}.txt"))).unwrap(),
            "original"
        );
    }
    resumed.kill().await.unwrap();
    resumed.wait().await.unwrap();
}
