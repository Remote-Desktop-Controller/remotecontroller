use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::{Value, json};
use std::sync::Arc;
#[cfg(windows)]
#[tokio::test]
async fn case_variant_cannot_modify_ignore_policy() {
    let (root, _state, k) = fixture().await;
    std::fs::write(root.path().join(".gitignore"), "secret\n").unwrap();
    std::fs::write(root.path().join("secret"), "private").unwrap();
    let result = run(
        &k,
        "filesystem.write",
        json!({"path":".GITIGNORE","content":""}),
    )
    .await;
    assert_eq!(result["status"], "Failed");
    assert_eq!(
        std::fs::read_to_string(root.path().join(".gitignore")).unwrap(),
        "secret\n"
    );
    assert!(k.services.files.read("secret").await.is_err());
}
struct LateCommit {
    files: runtime_infrastructure::files::WorkspaceFiles,
}
#[async_trait::async_trait]
impl Tool for LateCommit {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "late.commit".into(),
            version: "1".into(),
            description: "test fixture".into(),
            capabilities: vec![Capability::WriteWorkspace],
            input_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            risk: "write".into(),
            workload: Workload::FileWrite,
        }
    }
    async fn execute(&self, input: Value, ctx: ToolContext) -> runtime_ports::Result<Value> {
        self.files
            .write_atomic("committed", b"durable".to_vec())
            .await?;
        ctx.cancellation.cancel();
        tokio::task::yield_now().await;
        if input["fail"].as_bool().unwrap_or(false) {
            return Err(PortError::Tool("rollback failed; recovery required".into()));
        }
        Ok(json!({"committed":true}))
    }
}
#[tokio::test]
async fn cancellation_after_commit_preserves_successful_result() {
    let (_root, _state, k) = fixture().await;
    let mut registry = runtime_application::Registry::new();
    registry
        .register(Arc::new(LateCommit {
            files: k.services.files.clone(),
        }))
        .unwrap();
    let executor = runtime_application::Executor::new(
        k.services.store.clone(),
        registry,
        ExecutionPolicy::new([Capability::WriteWorkspace]),
        runtime_application::SchedulerLimits::default(),
    )
    .unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: k.workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "late.commit".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: now_ms(),
    };
    let op = executor
        .execute(
            op,
            now_ms() + 5000,
            Arc::new(runtime_infrastructure::kernel::SystemClock),
            Arc::new(NoProgress),
        )
        .await
        .unwrap();
    assert_eq!(op.status, OperationStatus::Succeeded);
    assert_eq!(
        serde_json::from_slice::<Value>(&op.result.unwrap().output).unwrap()["committed"],
        true
    );
}
#[tokio::test]
async fn cancellation_does_not_hide_cleanup_failure() {
    let (_root, _state, k) = fixture().await;
    let mut registry = runtime_application::Registry::new();
    registry
        .register(Arc::new(LateCommit {
            files: k.services.files.clone(),
        }))
        .unwrap();
    let executor = runtime_application::Executor::new(
        k.services.store.clone(),
        registry,
        ExecutionPolicy::new([Capability::WriteWorkspace]),
        runtime_application::SchedulerLimits::default(),
    )
    .unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: k.workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "late.commit".into(),
        input: br#"{"fail":true}"#.to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: now_ms(),
    };
    let result = executor
        .execute(
            op,
            now_ms() + 5000,
            Arc::new(runtime_infrastructure::kernel::SystemClock),
            Arc::new(NoProgress),
        )
        .await
        .unwrap();
    assert_eq!(result.status, OperationStatus::Failed);
    assert!(
        result
            .result
            .unwrap()
            .error
            .unwrap()
            .contains("rollback failed")
    );
}

async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<Kernel>) {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::WriteWorkspace,
            Capability::GitRead,
        ]),
    )
    .await
    .unwrap();
    (root, state, kernel)
}
fn call(k: &Kernel, tool: &str, input: Value) -> RequestEnvelope {
    RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 30_000),
        payload: Request::Execute {
            tool: tool.into(),
            input,
        },
    }
}
async fn run(k: &Arc<Kernel>, tool: &str, input: Value) -> Value {
    k.handle(call(k, tool, input), Arc::new(NoProgress))
        .await
        .payload
        .unwrap()
}
#[tokio::test]
async fn duplicate_mutation_executes_once_and_conflicting_payload_is_rejected() {
    let (root, _state, k) = fixture().await;
    let req = call(
        &k,
        "filesystem.write",
        json!({"path":"once","content":"value"}),
    );
    let (a, b) = tokio::join!(
        k.handle(req.clone(), Arc::new(NoProgress)),
        k.handle(req.clone(), Arc::new(NoProgress))
    );
    assert_eq!(
        a.payload.unwrap()["operation_id"],
        b.payload.unwrap()["operation_id"]
    );
    assert_eq!(std::fs::read(root.path().join("once")).unwrap(), b"value");
    let events = k.services.store.recent(k.workspace.id, 100).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "mutated").count(), 1);
    let mut conflict = req;
    conflict.payload = Request::Execute {
        tool: "filesystem.write".into(),
        input: json!({"path":"once","content":"other"}),
    };
    assert_eq!(
        k.handle(conflict, Arc::new(NoProgress))
            .await
            .payload
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
#[tokio::test]
async fn binary_checkpoint_and_raw_event_are_restorable() {
    let (root, _state, k) = fixture().await;
    let bytes = vec![0, 255, 128, 13, 10, 17];
    std::fs::write(root.path().join("binary.dat"), &bytes).unwrap();
    let checkpoint = run(&k, "checkpoint.create", json!({"paths":["binary.dat"]})).await;
    assert_eq!(checkpoint["status"], "Succeeded");
    std::fs::write(root.path().join("binary.dat"), b"changed").unwrap();
    let rollback = run(
        &k,
        "checkpoint.rollback",
        json!({"checkpoint_id":checkpoint["result"]["checkpoint_id"]}),
    )
    .await;
    assert_eq!(rollback["status"], "Succeeded");
    assert_eq!(
        std::fs::read(root.path().join("binary.dat")).unwrap(),
        bytes
    );
    let events = k.services.store.recent(k.workspace.id, 10).await.unwrap();
    assert!(events.iter().all(|e| e.raw.is_empty()));
    assert!(
        !k.services
            .store
            .raw_event(k.workspace.id, events[0].id)
            .await
            .unwrap()
            .raw
            .is_empty()
    );
}
#[tokio::test]
async fn partial_batch_failure_restores_earlier_files() {
    let (root, _state, k) = fixture().await;
    std::fs::write(root.path().join("a"), "old").unwrap();
    let result = run(
        &k,
        "filesystem.batch_edit",
        json!({"edits":[{"path":"a","content":"new"},{"path":"missing/child","content":"new"}]}),
    )
    .await;
    assert_eq!(result["status"], "Failed");
    assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"old");
    assert!(
        k.services
            .store
            .pending_checkpoints(k.workspace.id)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn checkpoint_rollback_restores_existing_and_removes_created_files() {
    let (root, _state, k) = fixture().await;
    std::fs::write(root.path().join("a"), "original").unwrap();
    let result = run(
        &k,
        "filesystem.batch_edit",
        json!({"edits":[{"path":"a","content":"changed"},{"path":"b","content":"created"}]}),
    )
    .await;
    assert_eq!(result["status"], "Succeeded");
    let rollback = run(
        &k,
        "checkpoint.rollback",
        json!({"checkpoint_id":result["result"]["checkpoint_id"]}),
    )
    .await;
    assert_eq!(rollback["status"], "Succeeded");
    assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"original");
    assert!(!root.path().join("b").exists());
}
#[tokio::test]
async fn versioned_context_never_reuses_pre_mutation_projection() {
    let (_root, _state, k) = fixture().await;
    let first = k
        .services
        .context
        .compile(k.workspace.id, None, 2048)
        .await
        .unwrap();
    let cached = k
        .services
        .context
        .compile(k.workspace.id, None, 2048)
        .await
        .unwrap();
    assert_eq!(first, cached);
    run(
        &k,
        "filesystem.write",
        json!({"path":"context-file","content":"new"}),
    )
    .await;
    let second = k
        .services
        .context
        .compile(k.workspace.id, None, 2048)
        .await
        .unwrap();
    assert_ne!(first, second);
    assert!(second.contains("context-file"));
}
#[tokio::test]
async fn restart_restores_prepared_journal_and_marks_interrupted_operation() {
    let (root, state, k) = fixture().await;
    let workspace = k.workspace.id;
    std::fs::write(root.path().join("a"), "before").unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: workspace,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "filesystem.batch_edit".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: now_ms(),
    };
    k.services.store.claim(&op).await.unwrap();
    k.services.store.set_running(op.id).await.unwrap();
    k.services
        .store
        .save_checkpoint(&Checkpoint {
            id: CheckpointId::new(),
            workspace_id: workspace,
            operation_id: op.id,
            files: vec![FileSnapshot {
                path: "a".into(),
                content: Some(b"before".to_vec()),
            }],
            committed: false,
        })
        .await
        .unwrap();
    std::fs::write(root.path().join("a"), "torn batch").unwrap();
    drop(k);
    let restarted = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    assert_eq!(restarted.workspace.id, workspace);
    assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"before");
    assert_eq!(
        restarted
            .services
            .store
            .operation(op.id)
            .await
            .unwrap()
            .status,
        OperationStatus::Failed
    );
}
#[tokio::test]
async fn policy_and_payload_validation_fail_closed() {
    let (root, state, k) = fixture().await;
    drop(k);
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    assert_eq!(
        k.handle(
            call(&k, "filesystem.write", json!({"path":"x","content":"x"})),
            Arc::new(NoProgress)
        )
        .await
        .payload
        .unwrap_err()
        .code,
        ErrorCode::PolicyDenied
    );
    assert!(
        k.handle(
            call(&k, "filesystem.read", json!({"path":"x","unknown":1})),
            Arc::new(NoProgress)
        )
        .await
        .payload
        .is_err()
    );
    assert!(!root.path().join("x").exists());
}
#[tokio::test]
async fn rust_ast_query_and_git_read_tools_are_real() {
    let (root, _state, k) = fixture().await;
    std::fs::write(root.path().join("code.rs"), "fn hello() {}\n").unwrap();
    let ast=run(&k,"code.symbols",json!({"path":"code.rs","language":"rust","query":"(function_item name: (identifier) @name)"})).await;
    assert_eq!(ast["status"], "Succeeded");
    assert_eq!(ast["result"]["captures"][0]["capture"], "name");
    let repo = git2::Repository::init(root.path()).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("code.rs")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Fixture", "fixture@local.invalid").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    std::fs::write(root.path().join("code.rs"), "fn changed() {}\n").unwrap();
    assert_eq!(
        run(&k, "git.status", json!({})).await["status"],
        "Succeeded"
    );
    assert!(
        run(&k, "git.diff", json!({})).await["result"]["diff"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    assert_eq!(
        run(&k, "git.show", json!({"revision":"HEAD"})).await["result"]["summary"],
        "initial"
    );
}
