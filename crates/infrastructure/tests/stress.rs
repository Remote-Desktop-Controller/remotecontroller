use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;

struct Progress {
    tx: mpsc::Sender<usize>,
}
impl ProgressPort for Progress {
    fn emit(&self, done: usize, _: usize, _: &str) {
        let _ = self.tx.try_send(done);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_thousand_files_thousand_edits_cancel_rollback_restart() {
    let started = std::time::Instant::now();
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    for i in 0..10_000 {
        std::fs::write(
            root.path().join(format!("file-{i:05}.txt")),
            format!("original-{i}"),
        )
        .unwrap();
    }
    let policy = ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]);
    let config = RuntimeConfig {
        operation_timeout_ms: 180_000,
        ..RuntimeConfig::default()
    };
    let kernel = Kernel::open(root.path(), state.path(), config.clone(), policy.clone())
        .await
        .unwrap();
    let paths = kernel
        .services
        .files
        .scan(".", 20_000, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(paths.len(), 10_000);
    let mut reads = tokio::task::JoinSet::new();
    for i in 0..64 {
        let kernel = kernel.clone();
        reads.spawn(async move {
            kernel
                .services
                .files
                .read(&format!("file-{i:05}.txt"))
                .await
                .unwrap()
        });
    }
    while let Some(r) = reads.join_next().await {
        assert!(r.unwrap().starts_with(b"original-"));
    }
    let edits: Vec<_> = (0..1000)
        .map(|i| json!({"path":format!("file-{i:05}.txt"),"content":format!("modified-{i}")}))
        .collect();
    let request = RequestEnvelope {
        meta: Metadata::new(Some(kernel.workspace.id.as_uuid()), 180_000),
        payload: Request::Execute {
            tool: "filesystem.batch_edit".into(),
            input: json!({"edits":edits}),
        },
    };
    let (tx, mut rx) = mpsc::channel(128);
    let cancel_request = request.clone();
    let cancel_kernel = kernel.clone();
    let worker = tokio::spawn(async move {
        cancel_kernel
            .handle(cancel_request, Arc::new(Progress { tx }))
            .await
            .payload
            .unwrap()
    });
    while rx.recv().await.unwrap() < 128 {}
    kernel
        .executor
        .cancel(
            kernel.workspace.id,
            request.meta.operation_id.to_string().parse().unwrap(),
        )
        .await
        .unwrap();
    let cancelled = worker.await.unwrap();
    assert_eq!(cancelled["status"], "Cancelled");
    for i in 0..1000 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("file-{i:05}.txt"))).unwrap(),
            format!("original-{i}")
        );
    }
    let mut second = request.clone();
    second.meta = Metadata::new(Some(kernel.workspace.id.as_uuid()), 180_000);
    let (tx, mut rx) = mpsc::channel(128);
    let result = kernel
        .handle(second.clone(), Arc::new(Progress { tx }))
        .await
        .payload
        .unwrap();
    assert_eq!(result["status"], "Succeeded");
    assert!(rx.try_recv().is_ok());
    for i in 0..1000 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("file-{i:05}.txt"))).unwrap(),
            format!("modified-{i}")
        );
    }
    let rollback = RequestEnvelope {
        meta: Metadata::new(Some(kernel.workspace.id.as_uuid()), 180_000),
        payload: Request::Execute {
            tool: "checkpoint.rollback".into(),
            input: json!({"checkpoint_id":result["result"]["checkpoint_id"]}),
        },
    };
    assert_eq!(
        kernel
            .handle(rollback, Arc::new(NoProgress))
            .await
            .payload
            .unwrap()["status"],
        "Succeeded"
    );
    let workspace = kernel.workspace.id;
    kernel.executor.shutdown().await;
    drop(kernel);
    let resumed = Kernel::open(root.path(), state.path(), config, policy)
        .await
        .unwrap();
    assert_eq!(resumed.workspace.id, workspace);
    assert_eq!(
        resumed
            .handle(second, Arc::new(NoProgress))
            .await
            .payload
            .unwrap()["operation_id"],
        result["operation_id"]
    );
    let context = resumed
        .services
        .context
        .compile(
            workspace,
            Some(request.meta.operation_id.to_string().parse().unwrap()),
            8192,
        )
        .await
        .unwrap();
    assert!(context.contains("Cancelled"));
    for i in 0..10_000 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("file-{i:05}.txt"))).unwrap(),
            format!("original-{i}")
        );
    }
    eprintln!(
        "stress files=10000 edits=1000 cancellation+rollback+restart elapsed_ms={}",
        started.elapsed().as_millis()
    );
}
