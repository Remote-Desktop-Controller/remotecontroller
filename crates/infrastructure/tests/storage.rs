use runtime_domain::*;
use runtime_infrastructure::storage::LocalStore;
use runtime_ports::*;

#[tokio::test]
async fn operation_claim_is_durable_and_unique() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let store = LocalStore::open(&path).await.unwrap();
    let workspace = store.open_workspace("fixture").await.unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "filesystem.read".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    assert!(store.claim(&op).await.unwrap());
    assert!(!store.claim(&op).await.unwrap());
    store.set_running(op.id).await.unwrap();
    store
        .finish(
            op.id,
            OperationStatus::Succeeded,
            &ToolResult {
                output: b"42".to_vec(),
                error: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_workspace(workspace.id)
            .await
            .unwrap()
            .graph_version,
        2
    );
    assert_eq!(
        store.causal(workspace.id, op.id, 50).await.unwrap().len(),
        2
    );
    drop(store);
    let reopened = LocalStore::open(&path).await.unwrap();
    assert_eq!(
        reopened.operation(op.id).await.unwrap().status,
        OperationStatus::Succeeded
    );
}

#[tokio::test]
async fn causal_query_repeated_large_selection_is_ordered_bounded_and_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&dir.path().join("causal.db"))
        .await
        .unwrap();
    let workspace = store.open_workspace("causal").await.unwrap();
    let other = store.open_workspace("unrelated").await.unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "benchmark".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    store.claim(&op).await.unwrap();
    let mut ids = Vec::new();
    for i in 0..256 {
        let event = Event {
            id: EventId::new(),
            workspace_id: workspace.id,
            operation_id: op.id,
            kind: "benchmark".into(),
            summary: format!("event {i}"),
            raw: vec![7; 32],
            created_at: i + 2,
        };
        store
            .append(&event, None, EdgeKind::Produced)
            .await
            .unwrap();
        ids.push(event.id);
    }
    for _ in 0..32 {
        let events = store.causal(workspace.id, op.id, 128).await.unwrap();
        assert_eq!(events.len(), 128);
        assert_eq!(
            events.iter().map(|e| e.id).collect::<Vec<_>>(),
            ids[128..].iter().rev().copied().collect::<Vec<_>>()
        );
        assert!(
            events
                .iter()
                .all(|e| e.workspace_id == workspace.id && e.raw.is_empty())
        );
    }
    assert!(
        store
            .causal(workspace.id, op.id, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.causal(other.id, op.id, 128).await.unwrap().is_empty());
    assert_eq!(
        store.raw_event(workspace.id, ids[255]).await.unwrap().raw,
        vec![7; 32]
    );
}
