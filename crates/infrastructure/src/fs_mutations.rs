use crate::{
    files::{Mutation, WorkspaceFiles},
    storage::{LocalStore, storage_error},
};
use runtime_domain::*;
use runtime_ports::*;
use serde_json::{Value, json};
pub(crate) async fn recover(
    files: &WorkspaceFiles,
    store: &LocalStore,
    workspace: WorkspaceId,
) -> Result<()> {
    let pending=store.run(move|c|async move{
        let mut rows=c.query("SELECT id,payload FROM filesystem_journal WHERE workspace_id=?1 AND status='prepared' ORDER BY created_at",[workspace.to_string()]).await.map_err(storage_error)?;
        let mut pending=Vec::new();while let Some(row)=rows.next().await.map_err(storage_error)?{pending.push((row.get::<String>(0).map_err(storage_error)?,row.get::<Vec<u8>>(1).map_err(storage_error)?));}Ok(pending)
    }).await?;
    for (id, payload) in pending {
        let mutation: Mutation = serde_json::from_slice(&payload).map_err(storage_error)?;
        files
            .blocking(move |files| files.rollback_mutation(&mutation))
            .await?;
        store
            .run(move |c| async move {
                c.execute(
                    "UPDATE filesystem_journal SET status='rolled_back' WHERE id=?1",
                    [id],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}
pub(crate) async fn execute(
    files: &WorkspaceFiles,
    store: &LocalStore,
    context: &ToolContext,
    name: &str,
    input: &Value,
) -> Result<Value> {
    let _lock = files.gate.write().await;
    let path = input["path"].as_str().unwrap_or("").to_owned();
    let source = input["source"].as_str().unwrap_or("").to_owned();
    let destination = input["destination"].as_str().unwrap_or("").to_owned();
    let operation = context.operation_id;
    let directory = name == "filesystem.mkdir";
    let mutation = files
        .blocking(move |files| {
            if directory {
                files.prepare_directory(&path, operation)
            } else {
                files.prepare_move(&source, &destination)
            }
        })
        .await?;
    let payload = serde_json::to_vec(&mutation).map_err(storage_error)?;
    let workspace = context.workspace_id;
    let prepared = store
        .run(move |c| async move {
            c.execute(
                "INSERT INTO filesystem_journal VALUES(?1,?2,?3,'prepared',?4)",
                libsql::params![
                    operation.to_string(),
                    workspace.to_string(),
                    payload,
                    runtime_protocol::now_ms() as i64
                ],
            )
            .await
            .map_err(storage_error)?;
            Ok(())
        })
        .await;
    if let Err(error) = prepared {
        let rollback = mutation.clone();
        files
            .blocking(move |files| files.rollback_mutation(&rollback))
            .await?;
        return Err(error);
    }
    let apply = mutation.clone();
    let token = context.cancellation.clone();
    let result = files
        .blocking(move |files| {
            if token.is_cancelled() {
                return Err(PortError::Cancelled);
            }
            files.apply_mutation(&apply)
        })
        .await;
    if let Err(error) = result {
        let rollback = mutation.clone();
        files
            .blocking(move |files| files.rollback_mutation(&rollback))
            .await?;
        store
            .run(move |c| async move {
                c.execute(
                    "UPDATE filesystem_journal SET status='rolled_back' WHERE id=?1",
                    [operation.to_string()],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await?;
        return Err(error);
    }
    let committed = store
        .run(move |c| async move {
            c.execute(
                "UPDATE filesystem_journal SET status='committed' WHERE id=?1",
                [operation.to_string()],
            )
            .await
            .map_err(storage_error)?;
            Ok(())
        })
        .await;
    if let Err(error) = committed {
        let rollback = mutation.clone();
        files
            .blocking(move |files| files.rollback_mutation(&rollback))
            .await?;
        store
            .run(move |c| async move {
                c.execute(
                    "UPDATE filesystem_journal SET status='rolled_back' WHERE id=?1",
                    [operation.to_string()],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await?;
        return Err(error);
    }
    Ok(
        json!({"journal_id":operation.to_string(),"committed":true,"path":input["path"],"source":input["source"],"destination":input["destination"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn journal(
        store: &LocalStore,
        workspace: WorkspaceId,
        mutation: &Mutation,
        op: OperationId,
    ) {
        store
            .claim(&Operation {
                id: op,
                workspace_id: workspace,
                task_id: None,
                trace_id: TraceId::new(),
                tool: "filesystem.mutation".into(),
                input: b"{}".to_vec(),
                status: OperationStatus::Queued,
                result: None,
                created_at: 1,
            })
            .await
            .unwrap();
        let payload = serde_json::to_vec(mutation).unwrap();
        store
            .run(move |c| async move {
                c.execute(
                    "INSERT INTO filesystem_journal VALUES(?1,?2,?3,'prepared',1)",
                    libsql::params![op.to_string(), workspace.to_string(), payload],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn directory_crash_after_effect_recovers_only_owned_empty_directory() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let files = WorkspaceFiles::open(root.path(), crate::files::FileLimits::default()).unwrap();
        let store = LocalStore::open(&state.path().join("runtime.db"))
            .await
            .unwrap();
        let workspace = store
            .open_workspace(&root.path().to_string_lossy())
            .await
            .unwrap();
        let op = OperationId::new();
        let mutation = files.prepare_directory("new", op).unwrap();
        journal(&store, workspace.id, &mutation, op).await;
        files.apply_mutation(&mutation).unwrap();
        assert!(root.path().join("new").exists());
        recover(&files, &store, workspace.id).await.unwrap();
        assert!(!root.path().join("new").exists());
        let op = OperationId::new();
        let mutation = files.prepare_directory("external", op).unwrap();
        journal(&store, workspace.id, &mutation, op).await;
        files.apply_mutation(&mutation).unwrap();
        std::fs::write(root.path().join("external/user"), "preserve").unwrap();
        assert!(matches!(
            recover(&files, &store, workspace.id).await,
            Err(PortError::Conflict)
        ));
        assert_eq!(
            std::fs::read_to_string(root.path().join("external/user")).unwrap(),
            "preserve"
        );
    }
    #[tokio::test]
    async fn moved_file_crash_rolls_back_metadata_and_preserves_external_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let files = WorkspaceFiles::open(root.path(), crate::files::FileLimits::default()).unwrap();
        let store = LocalStore::open(&state.path().join("runtime.db"))
            .await
            .unwrap();
        let ws = store
            .open_workspace(&root.path().to_string_lossy())
            .await
            .unwrap();
        std::fs::write(root.path().join("source"), "original").unwrap();
        let original = files.snapshot_sync("source").unwrap();
        let op = OperationId::new();
        let mutation = files.prepare_move("source", "destination").unwrap();
        journal(&store, ws.id, &mutation, op).await;
        files.apply_mutation(&mutation).unwrap();
        recover(&files, &store, ws.id).await.unwrap();
        assert!(!root.path().join("destination").exists());
        files.check_snapshot_sync(&original).unwrap();
        let op = OperationId::new();
        let mutation = files.prepare_move("source", "destination").unwrap();
        journal(&store, ws.id, &mutation, op).await;
        files.apply_mutation(&mutation).unwrap();
        std::fs::write(root.path().join("source"), "external").unwrap();
        assert!(matches!(
            recover(&files, &store, ws.id).await,
            Err(PortError::Conflict)
        ));
        assert_eq!(
            std::fs::read_to_string(root.path().join("source")).unwrap(),
            "external"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("destination")).unwrap(),
            "original"
        );
    }
}
