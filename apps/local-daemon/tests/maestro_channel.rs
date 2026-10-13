//! Fase 5: o canal `\\.\pipe\rdc-agent-maestro` executa o plano aprovado pelo operador
//! através do próprio kernel do daemon, e respeita a política de capacidades.
//!
//! Os dois cenários rodam em sequência dentro do mesmo teste para não disputar o pipe de
//! telemetria padrão (nome fixo) entre dois daemons simultâneos:
//! 1. daemon com `--allow-write` grava fisicamente um arquivo no workspace e devolve ack;
//! 2. daemon sem `--allow-write` recusa a mesma ordem com `PolicyDenied` e não toca no disco.
#![cfg(windows)]
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;

const ACK_TIMEOUT: Duration = Duration::from_secs(25);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn binary() -> PathBuf {
    std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join(format!("local-daemon{}", std::env::consts::EXE_SUFFIX)))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_local-daemon")))
}

struct TestDaemon {
    child: Child,
    state_dir: PathBuf,
    workspace: PathBuf,
    pipe: String,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TestDaemon {
    fn log(&self) -> String {
        std::fs::read_to_string(self.state_dir.join("daemon.log")).unwrap_or_default()
    }
}

/// Sobe um daemon isolado (workspace, estado e pipes próprios).
fn spawn_daemon(root: &Path, extra: &[&str]) -> TestDaemon {
    let workspace = root.join("workspace");
    let state_dir = root.join("state");
    let ipc_endpoint = format!(r"\\.\pipe\rdc-test-maestro-ipc-{}", uuid::Uuid::new_v4().simple());
    let pipe = format!(r"\\.\pipe\rdc-test-maestro-{}", uuid::Uuid::new_v4().simple());
    std::fs::create_dir_all(&workspace).unwrap();

    let child = Command::new(binary())
        .arg("serve")
        .arg("--workspace")
        .arg(&workspace)
        .arg("--state-dir")
        .arg(&state_dir)
        .arg("--endpoint")
        .arg(&ipc_endpoint)
        .arg("--maestro-pipe")
        .arg(&pipe)
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("falha ao iniciar o daemon de teste");

    let mut daemon = TestDaemon {
        child,
        state_dir,
        workspace,
        pipe,
    };

    // Um flag rejeitado pelo clap encerra o processo imediatamente.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(status) = daemon.child.try_wait().unwrap() {
            let mut stderr = String::new();
            if let Some(mut pipe) = daemon.child.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            panic!("daemon encerrou cedo ({status}): {stderr}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    daemon
}

/// Conecta ao canal do Maestro, envia a ordem e devolve o recibo bruto.
async fn dispatch_order(pipe: &str, order: &str) -> String {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let client = loop {
        match ClientOptions::new().open(pipe) {
            Ok(client) => break client,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "canal do Maestro nunca ficou disponível em {pipe}: {error}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };

    let mut client = client;
    client
        .write_all(format!("{order}\n").as_bytes())
        .await
        .expect("falha ao escrever a ordem");
    client.flush().await.expect("falha ao liberar a ordem");

    let mut reader = BufReader::new(client);
    let mut ack = String::new();
    tokio::time::timeout(ACK_TIMEOUT, reader.read_line(&mut ack))
        .await
        .expect("timeout aguardando o ack do daemon")
        .expect("falha ao ler o ack do daemon");
    ack
}

fn write_order(path: &str, content: &str) -> String {
    format!(
        r#"{{"schema":"rdc.maestro.order.v1","source":"rdc-maestro","kind":"execute_approved_plan","issued_at":1,"mission_id":"m-e2e","payload_encoding":"json","payload":{{"steps":[{{"tool":"filesystem.write","input":{{"path":"{path}","content":"{content}"}}}}]}}}}"#
    )
}

#[test]
fn maestro_channel_executes_and_gates_orders() {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // ---- Cenário 1: com capacidade de escrita, a ordem é fisicamente aplicada.
    let granted = tempfile::tempdir().unwrap();
    let daemon = spawn_daemon(granted.path(), &["--allow-write"]);
    let ack = runtime.block_on(dispatch_order(
        &daemon.pipe,
        &write_order("maestro-ack.txt", "aplicado pelo maestro"),
    ));
    let parsed: serde_json::Value =
        serde_json::from_str(ack.trim()).expect("ack do daemon não é JSON válido");

    assert_eq!(parsed["schema"], "rdc.maestro.ack.v1", "ack: {parsed}");
    assert_eq!(parsed["status"], "completed", "ack: {parsed}");
    assert_eq!(parsed["mission_id"], "m-e2e");
    assert_eq!(parsed["steps_executed"], 1);
    assert_eq!(parsed["steps"][0]["tool"], "filesystem.write");
    assert_eq!(parsed["steps"][0]["status"], "ok");

    let written = daemon.workspace.join("maestro-ack.txt");
    assert!(
        written.exists(),
        "o plano aprovado não foi aplicado no disco: {}\nlog: {}",
        written.display(),
        daemon.log()
    );
    assert_eq!(
        std::fs::read_to_string(&written).unwrap(),
        "aplicado pelo maestro"
    );
    assert!(
        daemon.log().contains("maestro order channel listening"),
        "o daemon deveria registrar o canal do Maestro no log: {}",
        daemon.log()
    );
    drop(daemon);

    // ---- Cenário 2: sem `--allow-write`, a política do kernel recusa a ordem.
    let denied = tempfile::tempdir().unwrap();
    let daemon = spawn_daemon(denied.path(), &[]);
    let ack = runtime.block_on(dispatch_order(
        &daemon.pipe,
        &write_order("bloqueado.txt", "nao deveria gravar"),
    ));
    let parsed: serde_json::Value =
        serde_json::from_str(ack.trim()).expect("ack do daemon não é JSON válido");

    assert_eq!(parsed["status"], "failed", "ack: {parsed}");
    assert_eq!(
        parsed["failure"]["error"]["code"], "PolicyDenied",
        "ack: {parsed}"
    );
    assert_eq!(parsed["steps"][0]["status"], "failed");
    assert!(
        !daemon.workspace.join("bloqueado.txt").exists(),
        "a ordem foi aplicada apesar da política negar a escrita"
    );
}
