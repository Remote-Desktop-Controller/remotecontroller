use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel, processes::ProgramRule};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;

// Launched only as a separate child by the integration tests below.
#[test]
#[ignore = "process fixture, intentionally sleeps until killed"]
fn sleeper() {
    println!("child-ready");
    std::thread::sleep(std::time::Duration::from_secs(20));
}
struct Progress {
    tx: mpsc::Sender<String>,
}
impl ProgressPort for Progress {
    fn emit(&self, _: usize, _: usize, message: &str) {
        let _ = self.tx.try_send(message.into());
    }
}
async fn fixture(
    timeout: u64,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<Kernel>,
    String,
    Vec<String>,
) {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let args = vec![
        "--exact".into(),
        "sleeper".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    let config = RuntimeConfig {
        operation_timeout_ms: timeout,
        allowed_programs: vec![ProgramRule {
            program: executable.clone(),
            args: args.clone(),
        }],
        ..RuntimeConfig::default()
    };
    let kernel = Kernel::open(
        root.path(),
        state.path(),
        config,
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::SpawnProcess,
            Capability::KillProcess,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    (
        root,
        state,
        kernel,
        executable
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string(),
        args,
    )
}
#[tokio::test]
async fn operation_cancellation_kills_child_and_records_cancelled() {
    let (_root, _state, k, program, args) = fixture(10_000).await;
    let request = RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
        payload: Request::Execute {
            tool: "process.spawn".into(),
            input: json!({"program":program,"args":args}),
        },
    };
    let operation = request.meta.operation_id.to_string().parse().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let worker_k = k.clone();
    let worker =
        tokio::spawn(async move { worker_k.handle(request, Arc::new(Progress { tx })).await });
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let process_id = message
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("process_id=")
        .parse()
        .unwrap();
    assert_eq!(
        k.services
            .processes
            .status(k.workspace.id, process_id, None)
            .await
            .unwrap()["status"],
        "Running"
    );
    k.executor.cancel(k.workspace.id, operation).await.unwrap();
    assert_eq!(
        worker.await.unwrap().payload.unwrap()["status"],
        "Cancelled"
    );
    assert_eq!(
        k.services
            .processes
            .status(k.workspace.id, process_id, None)
            .await
            .unwrap()["status"],
        "Cancelled"
    );
    assert!(
        k.services
            .processes
            .kill(k.workspace.id, process_id)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn deadline_times_out_child_and_unlisted_arguments_are_denied() {
    let (_root, _state, k, program, args) = fixture(250).await;
    let result = k
        .handle(
            RequestEnvelope {
                meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
                payload: Request::Execute {
                    tool: "process.spawn".into(),
                    input: json!({"program":program,"args":args}),
                },
            },
            Arc::new(NoProgress),
        )
        .await
        .payload
        .unwrap();
    assert_eq!(result["status"], "TimedOut");
    let denied = k
        .handle(
            RequestEnvelope {
                meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
                payload: Request::Execute {
                    tool: "process.spawn".into(),
                    input: json!({"program":program,"args":["unlisted"]}),
                },
            },
            Arc::new(NoProgress),
        )
        .await
        .payload
        .unwrap();
    assert_eq!(denied["status"], "Failed");
}
#[tokio::test]
async fn scheduler_applies_backpressure_to_live_operations() {
    let (root, state, k, program, args) = fixture(10_000).await;
    let mut config = k.config.clone();
    k.executor.shutdown().await;
    drop(k);
    config.scheduler.queue = 1;
    config.scheduler.processes = 1;
    let k = Kernel::open(
        root.path(),
        state.path(),
        config,
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::SpawnProcess,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    let request = RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
        payload: Request::Execute {
            tool: "process.spawn".into(),
            input: json!({"program":program,"args":args}),
        },
    };
    let operation = request.meta.operation_id.to_string().parse().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let worker_k = k.clone();
    let request2 = request.clone();
    let worker =
        tokio::spawn(async move { worker_k.handle(request, Arc::new(Progress { tx })).await });
    rx.recv().await.unwrap();
    let mut request2 = request2;
    request2.meta = Metadata::new(Some(k.workspace.id.as_uuid()), 10_000);
    assert_eq!(
        k.handle(request2, Arc::new(NoProgress))
            .await
            .payload
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    k.executor.cancel(k.workspace.id, operation).await.unwrap();
    assert_eq!(
        worker.await.unwrap().payload.unwrap()["status"],
        "Cancelled"
    );
}
