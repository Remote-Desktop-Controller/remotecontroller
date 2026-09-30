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
