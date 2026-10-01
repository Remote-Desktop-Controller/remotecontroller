use runtime_domain::*;
use runtime_infrastructure::{
    context::{ContextEngine, ContextRequest},
    maintenance::{Maintenance, RetentionPolicy},
    storage::LocalStore,
};
use runtime_ports::*;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn query_options_change_selection_and_cache() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&dir.path().join("state.db"))
            .await
            .unwrap(),
    );
    let ws = store.open_workspace("fixture").await.unwrap();
    for summary in ["src/auth.rs login token", "src/video.rs render timeline"] {
        let op = Operation {
            id: OperationId::new(),
            workspace_id: ws.id,
            task_id: None,
            trace_id: TraceId::new(),
            tool: "read".into(),
            input: vec![],
            status: OperationStatus::Queued,
            result: None,
            created_at: 1,
        };
        store.claim(&op).await.unwrap();
        store
            .append(
                &Event {
                    id: EventId::new(),
                    workspace_id: ws.id,
                    operation_id: op.id,
                    kind: "decision".into(),
                    summary: summary.into(),
                    raw: vec![],
                    created_at: 2,
                },
                None,
                EdgeKind::FollowedBy,
            )
            .await
            .unwrap();
    }
    let engine = ContextEngine::new(store, 4, Duration::from_secs(60));
    let req = |q: &str| ContextRequest {
        workspace: ws.id,
        operation: None,
        task_id: None,
        query: q.into(),
        budget: 230,
    };
    let auth = engine.compile_request(req("auth login")).await.unwrap();
    let video = engine.compile_request(req("video render")).await.unwrap();
    assert!(auth.contains("src/auth.rs"), "{auth}");
    assert!(video.contains("src/video.rs"), "{video}");
    assert_ne!(auth, video);
    assert!(auth.len() <= 230);
    assert_eq!(
        engine
            .metrics
            .misses
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
}

#[tokio::test]
async fn backup_reopens_and_retention_preserves_prepared_and_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("user.txt");
    std::fs::write(&workspace, b"precious").unwrap();
    let store = LocalStore::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let ws = store
        .open_workspace(dir.path().to_str().unwrap())
        .await
        .unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: ws.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "write".into(),
        input: vec![],
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    store.claim(&op).await.unwrap();
    let cp = Checkpoint {
        id: CheckpointId::new(),
        workspace_id: ws.id,
        operation_id: op.id,
        files: vec![FileSnapshot {
            path: workspace.to_string_lossy().into(),
            content: Some(b"precious".to_vec()),
            metadata: None,
            restore_precondition: None,
        }],
        committed: false,
    };
    store.save_checkpoint(&cp).await.unwrap();
    let m = Maintenance::new(store.clone());
    let destination = dir.path().join("backup.db");
    m.backup(&destination).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let backup = LocalStore::open(&destination).await.unwrap();
    assert_eq!(backup.pending_checkpoints(ws.id).await.unwrap().len(), 1);
    let policy = RetentionPolicy {
        older_than_ms: u64::MAX,
        max_records: 10,
    };
    assert!(m.run(&policy, false, None).await.is_err());
    let report = m.run(&policy, true, None).await.unwrap();
    assert!(report.checkpoint_ids.is_empty());
    assert!(store.checkpoint(cp.id).await.is_ok());
    assert_eq!(std::fs::read(workspace).unwrap(), b"precious");
}

#[test]
fn unicode_budget_never_splits_codepoint() {
    use runtime_infrastructure::context::compile_events;
    let ws = WorkspaceId::new();
    let e = Event {
        id: EventId::new(),
        workspace_id: ws,
        operation_id: OperationId::new(),
        kind: "Failed".into(),
        summary: "ação 日本語 🦀".into(),
        raw: vec![],
        created_at: 1,
    };
    for budget in 0..220 {
        let out = compile_events(ws, 1, std::slice::from_ref(&e), budget);
        assert!(out.len() <= budget);
    }
}

#[tokio::test]
async fn selected_task_precedes_unrelated_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&dir.path().join("state.db"))
            .await
            .unwrap(),
    );
    let ws = store.open_workspace("fixture").await.unwrap();
    let task = Task {
        id: TaskId::new(),
        workspace_id: ws.id,
        label: "repair auth".into(),
    };
    store.save_task(&task).await.unwrap();
    for (task_id, kind, summary) in [
        (Some(task.id), "decision", "current task auth"),
        (None, "Failed", "unrelated failure"),
    ] {
        let op = Operation {
            id: OperationId::new(),
            workspace_id: ws.id,
            task_id,
            trace_id: TraceId::new(),
            tool: "read".into(),
            input: vec![],
            status: OperationStatus::Queued,
            result: None,
            created_at: 1,
        };
        store.claim(&op).await.unwrap();
        store
            .append(
                &Event {
                    id: EventId::new(),
                    workspace_id: ws.id,
                    operation_id: op.id,
                    kind: kind.into(),
                    summary: summary.into(),
                    raw: vec![],
                    created_at: 2,
                },
                None,
                EdgeKind::FollowedBy,
            )
            .await
            .unwrap();
    }
    let engine = ContextEngine::new(store, 4, Duration::from_secs(60));
    let out = engine
        .compile_request(ContextRequest {
            workspace: ws.id,
            operation: None,
            task_id: Some(task.id),
            query: String::new(),
            budget: 230,
        })
        .await
        .unwrap();
    assert!(out.contains("current task auth"), "{out}");
}

#[tokio::test]
async fn only_old_rolled_back_terminal_checkpoints_are_retained_out() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let ws = store.open_workspace("fixture").await.unwrap();
    let mut ids = Vec::new();
    for terminal in [true, false] {
        let op = Operation {
            id: OperationId::new(),
            workspace_id: ws.id,
            task_id: None,
            trace_id: TraceId::new(),
            tool: "write".into(),
            input: vec![],
            status: OperationStatus::Queued,
            result: None,
            created_at: 1,
        };
        store.claim(&op).await.unwrap();
        let cp = Checkpoint {
            id: CheckpointId::new(),
            workspace_id: ws.id,
            operation_id: op.id,
            files: vec![],
            committed: false,
        };
        store.save_checkpoint(&cp).await.unwrap();
        store.rollback_recorded(cp.id).await.unwrap();
        if terminal {
            store.set_running(op.id).await.unwrap();
            store
                .finish(
                    op.id,
                    OperationStatus::Succeeded,
                    &ToolResult {
                        output: vec![],
                        error: None,
                    },
                )
                .await
                .unwrap();
        }
        ids.push(cp.id);
    }
    let m = Maintenance::new(store.clone());
    let policy = RetentionPolicy {
        older_than_ms: u64::MAX,
        max_records: 10,
    };
    let dry = m.plan(&policy).await.unwrap();
    assert_eq!(dry.checkpoint_ids, vec![ids[0].to_string()]);
    assert!(store.checkpoint(ids[0]).await.is_ok());
    let report = m
        .run(
            &policy,
            false,
            Some(&dir.path().join("retention-backup.db")),
        )
        .await
        .unwrap();
    assert_eq!(report.removed_records, 1);
    assert!(store.checkpoint(ids[0]).await.is_err());
    assert!(store.checkpoint(ids[1]).await.is_ok());
    assert!(!store.recent(ws.id, 10).await.unwrap().is_empty());
    let backup = LocalStore::open(&dir.path().join("retention-backup.db"))
        .await
        .unwrap();
    assert!(backup.checkpoint(ids[0]).await.is_ok());
}

#[tokio::test]
async fn process_rollups_use_real_metrics_and_raw_references() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalStore::open(&dir.path().join("state.db"))
            .await
            .unwrap(),
    );
    let ws = store.open_workspace("fixture").await.unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: ws.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "process.status".into(),
        input: vec![],
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    store.claim(&op).await.unwrap();
    store.set_running(op.id).await.unwrap();
    store.finish(op.id,OperationStatus::Succeeded,&ToolResult{output:br#"{"process_id":"p1","status":"Exited","duration_ms":1234,"stdout_bytes":81,"stderr_bytes":2,"exit_code":0}"#.to_vec(),error:None}).await.unwrap();
    let engine = ContextEngine::new(store, 4, Duration::from_secs(60));
    let out = engine
        .compile_request(ContextRequest {
            workspace: ws.id,
            operation: Some(op.id),
            task_id: None,
            query: "process".into(),
            budget: 4096,
        })
        .await
        .unwrap();
    for expected in [
        "process_rollup",
        "duration_ms=1234",
        "stdout_bytes=81",
        "stderr_bytes=2",
        "exit_code=0",
        "raw_event=",
    ] {
        assert!(out.contains(expected), "{out}");
    }
}

#[tokio::test]
async fn backup_preserves_spools_and_never_overwrites_destination() {
    let dir = tempfile::tempdir().unwrap();
    let spool = dir.path().join("process-spool").join("p1");
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join("stdout"), b"durable output").unwrap();
    let store = LocalStore::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let m = Maintenance::new(store);
    let destination = dir.path().join("backup.db");
    let info = m.backup(&destination).await.unwrap();
    assert_eq!(
        std::fs::read(info.spool_directory.unwrap().join("p1/stdout")).unwrap(),
        b"durable output"
    );
    assert!(m.backup(&destination).await.is_err());
    assert_eq!(
        std::fs::read(spool.join("stdout")).unwrap(),
        b"durable output"
    );
}
