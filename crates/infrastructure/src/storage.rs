//! Embedded libSQL. Blocking local engine calls run off Tokio I/O workers.
use async_trait::async_trait;
use libsql::{Connection, Database, params};
use runtime_domain::*;
use runtime_ports::*;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

pub(crate) fn storage_error(e: impl std::fmt::Display) -> PortError {
    PortError::Storage(e.to_string())
}
#[derive(Clone)]
pub struct LocalStore {
    db: Arc<Database>,
    workers: Arc<Semaphore>,
    pub metrics: Arc<StorageMetrics>,
}
#[derive(Default)]
pub struct StorageMetrics {
    pub calls: AtomicU64,
    pub elapsed_micros: AtomicU64,
}
impl LocalStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_owned();
        let handle = tokio::runtime::Handle::current();
        let db = tokio::task::spawn_blocking(move || {
            handle.block_on(async {
                libsql::Builder::new_local(path)
                    .build()
                    .await
                    .map_err(storage_error)
            })
        })
        .await
        .map_err(storage_error)??;
        let store = Self {
            db: Arc::new(db),
            workers: Arc::new(Semaphore::new(4)),
            metrics: Arc::new(StorageMetrics::default()),
        };
        store
            .run(|c| async move {
                c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
                    .await
                    .map_err(storage_error)?;
                let tx = c
                    .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                    .await
                    .map_err(storage_error)?;
                tx.execute_batch(include_str!("../../../migrations/001_initial.sql"))
                    .await
                    .map_err(storage_error)?;
                tx.execute(
                    "INSERT OR IGNORE INTO schema_migrations VALUES(1,?1)",
                    [runtime_protocol::now_ms() as i64],
                )
                .await
                .map_err(storage_error)?;
                tx.commit().await.map_err(storage_error)
            })
            .await?;
        Ok(store)
    }
    pub(crate) async fn run<T, F, Fut>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Connection) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let started = std::time::Instant::now();
        let permit = self
            .workers
            .clone()
            .acquire_owned()
            .await
            .map_err(storage_error)?;
        let db = self.db.clone();
        let handle = tokio::runtime::Handle::current();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let connection = db.connect().map_err(storage_error)?;
            connection
                .busy_timeout(Duration::from_secs(5))
                .map_err(storage_error)?;
            handle.block_on(async {
                connection
                    .execute("PRAGMA foreign_keys=ON", ())
                    .await
                    .map_err(storage_error)?;
                f(connection).await
            })
        })
        .await
        .map_err(storage_error)?;
        self.metrics.calls.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .elapsed_micros
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        result
    }
    pub async fn rollback_recorded(&self, id: CheckpointId) -> Result<()> {
        let id = id.to_string();
        self.run(move |c| async move {
            c.execute(
                "UPDATE checkpoints SET status='rolled_back',updated_at=?2 WHERE id=?1",
                params![id, runtime_protocol::now_ms() as i64],
            )
            .await
            .map_err(storage_error)?;
            Ok(())
        })
        .await
    }
}
fn status(s: &str) -> Result<OperationStatus> {
    match s {
        "Queued" => Ok(OperationStatus::Queued),
        "Running" => Ok(OperationStatus::Running),
        "Succeeded" => Ok(OperationStatus::Succeeded),
        "Failed" => Ok(OperationStatus::Failed),
        "Cancelled" => Ok(OperationStatus::Cancelled),
        "TimedOut" => Ok(OperationStatus::TimedOut),
        _ => Err(storage_error("unknown persisted operation state")),
    }
}
fn operation_row(row: &libsql::Row) -> Result<Operation> {
    let task: Option<String> = row.get(2).map_err(storage_error)?;
    let output: Option<Vec<u8>> = row.get(7).map_err(storage_error)?;
    Ok(Operation {
        id: row
            .get::<String>(0)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        workspace_id: row
            .get::<String>(1)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        task_id: task.map(|s| s.parse()).transpose().map_err(storage_error)?,
        trace_id: row
            .get::<String>(3)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        tool: row.get(4).map_err(storage_error)?,
        input: row.get(5).map_err(storage_error)?,
        status: status(&row.get::<String>(6).map_err(storage_error)?)?,
        result: output.map(|output| ToolResult {
            output,
            error: row.get(8).ok().flatten(),
        }),
        created_at: row.get::<i64>(9).map_err(storage_error)? as u64,
    })
}
const OP_COLUMNS: &str =
    "id,workspace_id,task_id,trace_id,tool,input,status,output,error,created_at";

#[async_trait]
impl WorkspaceRepository for LocalStore {
    async fn open_workspace(&self, root: &str) -> Result<Workspace> {
        let root = root.to_owned();
        self.run(move |c| async move {
            let id = WorkspaceId::new(); let now = runtime_protocol::now_ms() as i64;
            c.execute("INSERT OR IGNORE INTO workspaces(id,root,created_at,updated_at) VALUES(?1,?2,?3,?3)", params![id.to_string(), root.clone(), now]).await.map_err(storage_error)?;
            let mut rows = c.query("SELECT id,graph_version FROM workspaces WHERE root=?1", [root.clone()]).await.map_err(storage_error)?;
            let row = rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?;
            Ok(Workspace { id: row.get::<String>(0).map_err(storage_error)?.parse().map_err(storage_error)?, root, graph_version: row.get::<i64>(1).map_err(storage_error)? as u64 })
        }).await
    }
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace> {
        self.run(move |c| async move {
            let mut rows = c
                .query(
                    "SELECT root,graph_version FROM workspaces WHERE id=?1",
                    [id.to_string()],
                )
                .await
                .map_err(storage_error)?;
            let row = rows
                .next()
                .await
                .map_err(storage_error)?
                .ok_or(PortError::NotFound)?;
            Ok(Workspace {
                id,
                root: row.get(0).map_err(storage_error)?,
                graph_version: row.get::<i64>(1).map_err(storage_error)? as u64,
            })
        })
        .await
    }
}
#[async_trait]
impl TaskRepository for LocalStore {
    async fn save_task(&self, task: &Task) -> Result<()> {
        let task = task.clone();
        self.run(move |c| async move {
            let now = runtime_protocol::now_ms() as i64;
            c.execute("INSERT OR IGNORE INTO tasks(id,workspace_id,label,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)", params![task.id.to_string(), task.workspace_id.to_string(), task.label, now]).await.map_err(storage_error)?;
            let mut rows = c.query("SELECT workspace_id FROM tasks WHERE id=?1", [task.id.to_string()]).await.map_err(storage_error)?;
            if rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?.get::<String>(0).map_err(storage_error)? != task.workspace_id.to_string() { return Err(PortError::Conflict); }
            Ok(())
        }).await
    }
}
async fn add_event(
    c: &Connection,
    event: &Event,
    parent: Option<EventId>,
    kind: EdgeKind,
) -> Result<()> {
    c.execute("INSERT INTO events(id,workspace_id,operation_id,type,summary,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![event.id.to_string(), event.workspace_id.to_string(), event.operation_id.to_string(), event.kind.clone(), event.summary.clone(), event.raw.clone(), event.created_at as i64]).await.map_err(storage_error)?;
    c.execute(
        "INSERT INTO event_nodes VALUES(?1,?2,?3)",
        params![
            event.id.to_string(),
            event.workspace_id.to_string(),
            event.operation_id.to_string()
        ],
    )
    .await
    .map_err(storage_error)?;
    if let Some(parent) = parent {
        c.execute(
            "INSERT INTO event_edges VALUES(?1,?2,?3)",
            params![
                parent.to_string(),
                event.id.to_string(),
                format!("{kind:?}")
            ],
        )
        .await
        .map_err(storage_error)?;
    }
    c.execute(
        "UPDATE workspaces SET graph_version=graph_version+1,updated_at=?2 WHERE id=?1",
        params![event.workspace_id.to_string(), event.created_at as i64],
    )
    .await
    .map_err(storage_error)?;
    Ok(())
}
#[async_trait]
impl OperationRepository for LocalStore {
    async fn claim(&self, op: &Operation) -> Result<bool> {
        let op = op.clone();
        self.run(move |c| async move {
            let tx = c.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await.map_err(storage_error)?;
            let changes = tx.execute("INSERT OR IGNORE INTO operations(id,workspace_id,task_id,trace_id,tool,input,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'Queued',?7,?7)",
                params![op.id.to_string(),op.workspace_id.to_string(),op.task_id.map(|id| id.to_string()),op.trace_id.to_string(),op.tool.clone(),op.input.clone(),op.created_at as i64]).await.map_err(storage_error)?;
            if changes == 0 { tx.rollback().await.map_err(storage_error)?; return Ok(false); }
            tx.execute("INSERT INTO tool_calls VALUES(?1,?2,?3,?4,?5)", params![ToolCallId::new().to_string(),op.workspace_id.to_string(),op.id.to_string(),op.tool.clone(),op.created_at as i64]).await.map_err(storage_error)?;
            let mut previous = tx.query("SELECT id FROM events WHERE workspace_id=?1 ORDER BY seq DESC LIMIT 1", [op.workspace_id.to_string()]).await.map_err(storage_error)?;
            let parent = previous.next().await.map_err(storage_error)?.map(|r| r.get::<String>(0).map_err(storage_error)?.parse().map_err(storage_error)).transpose()?;
            add_event(&tx, &Event { id: EventId::new(), workspace_id: op.workspace_id, operation_id: op.id,
                kind: "queued".into(), summary: format!("{} queued",op.tool), raw: op.input, created_at: op.created_at }, parent, EdgeKind::FollowedBy).await?;
            tx.commit().await.map_err(storage_error)?;
            Ok(true)
        }).await
    }
    async fn operation(&self, id: OperationId) -> Result<Operation> {
        self.run(move |c| async move {
            let mut rows = c
                .query(
                    &format!("SELECT {OP_COLUMNS} FROM operations WHERE id=?1"),
                    [id.to_string()],
                )
                .await
                .map_err(storage_error)?;
            operation_row(
                &rows
                    .next()
                    .await
                    .map_err(storage_error)?
                    .ok_or(PortError::NotFound)?,
            )
        })
        .await
    }
    async fn set_running(&self, id: OperationId) -> Result<()> {
        self.run(move |c| async move {
            let n = c.execute("UPDATE operations SET status='Running',updated_at=?2 WHERE id=?1 AND status='Queued'", params![id.to_string(),runtime_protocol::now_ms() as i64]).await.map_err(storage_error)?;
            if n != 1 { return Err(PortError::Conflict); } Ok(())
        }).await
    }
    async fn finish(
        &self,
        id: OperationId,
        new_status: OperationStatus,
        result: &ToolResult,
    ) -> Result<()> {
        let result = result.clone();
        self.run(move |c| async move {
            let tx = c
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await
                .map_err(storage_error)?;
            let mut rows = tx
                .query(
                    &format!("SELECT {OP_COLUMNS} FROM operations WHERE id=?1"),
                    [id.to_string()],
                )
                .await
                .map_err(storage_error)?;
            let op = operation_row(
                &rows
                    .next()
                    .await
                    .map_err(storage_error)?
                    .ok_or(PortError::NotFound)?,
            )?;
            op.status.transition(new_status).map_err(storage_error)?;
            let now = runtime_protocol::now_ms();
            tx.execute(
                "UPDATE operations SET status=?2,output=?3,error=?4,updated_at=?5 WHERE id=?1",
                params![
                    id.to_string(),
                    format!("{new_status:?}"),
                    result.output.clone(),
                    result.error.clone(),
                    now as i64
                ],
            )
            .await
            .map_err(storage_error)?;
            let mut rows = tx
                .query(
                    "SELECT id FROM events WHERE operation_id=?1 ORDER BY seq DESC LIMIT 1",
                    [id.to_string()],
                )
                .await
                .map_err(storage_error)?;
            let parent = rows
                .next()
                .await
                .map_err(storage_error)?
                .map(|r| {
                    r.get::<String>(0)
                        .map_err(storage_error)?
                        .parse()
                        .map_err(storage_error)
                })
                .transpose()?;
            add_event(
                &tx,
                &Event {
                    id: EventId::new(),
                    workspace_id: op.workspace_id,
                    operation_id: id,
                    kind: format!("{new_status:?}"),
                    summary: format!(
                        "{}: {new_status:?}; {} bytes; {}",
                        op.tool,
                        result.output.len(),
                        result.error.as_deref().unwrap_or("ok")
                    ),
                    raw: result.output,
                    created_at: now,
                },
                parent,
                EdgeKind::CausedBy,
            )
            .await?;
            tx.commit().await.map_err(storage_error)
        })
        .await
    }
    async fn interrupted(&self) -> Result<Vec<Operation>> {
        self.run(|c| async move {
            let mut rows = c
                .query(
                    &format!(
                        "SELECT {OP_COLUMNS} FROM operations WHERE status IN ('Queued','Running')"
                    ),
                    (),
                )
                .await
                .map_err(storage_error)?;
            let mut ops = Vec::new();
            while let Some(row) = rows.next().await.map_err(storage_error)? {
                ops.push(operation_row(&row)?);
            }
            Ok(ops)
        })
        .await
    }
}
fn event_row(row: &libsql::Row) -> Result<Event> {
    Ok(Event {
        id: row
            .get::<String>(0)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        workspace_id: row
            .get::<String>(1)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        operation_id: row
            .get::<String>(2)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        kind: row.get(3).map_err(storage_error)?,
        summary: row.get(4).map_err(storage_error)?,
        raw: row.get(5).map_err(storage_error)?,
        created_at: row.get::<i64>(6).map_err(storage_error)? as u64,
    })
}
#[async_trait]
impl EventRepository for LocalStore {
    async fn raw_event(&self, workspace: WorkspaceId, id: EventId) -> Result<Event> {
        self.run(move|c|async move{
            let mut rows=c.query("SELECT id,workspace_id,operation_id,type,summary,payload,created_at FROM events WHERE workspace_id=?1 AND id=?2",params![workspace.to_string(),id.to_string()]).await.map_err(storage_error)?;
            event_row(&rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?)
        }).await
    }
    async fn recent(&self, ws: WorkspaceId, limit: usize) -> Result<Vec<Event>> {
        self.run(move |c| async move {
            let mut rows = c.query("SELECT id,workspace_id,operation_id,type,summary,X'',created_at FROM events WHERE workspace_id=?1 ORDER BY seq DESC LIMIT ?2", params![ws.to_string(),limit.min(1000) as i64]).await.map_err(storage_error)?;
            let mut events = Vec::new(); while let Some(r) = rows.next().await.map_err(storage_error)? { events.push(event_row(&r)?); } Ok(events)
        }).await
    }
    async fn causal(&self, ws: WorkspaceId, op: OperationId, limit: usize) -> Result<Vec<Event>> {
        self.run(move |c| async move {
            // Bounded graph traversal avoids the recursive-CTE native crash reproduced
            // by optimized Windows GNU benchmarks on libSQL 0.9.30.
            use std::collections::{HashSet,VecDeque};
            let limit=limit.min(1000);
            if limit==0{return Ok(Vec::new());}
            let mut seeds=c.query("SELECT id FROM events WHERE operation_id=?1 AND workspace_id=?2 ORDER BY seq DESC LIMIT ?3",params![op.to_string(),ws.to_string(),limit as i64]).await.map_err(storage_error)?;
            let mut seen=HashSet::new();let mut frontier=VecDeque::new();
            while let Some(row)=seeds.next().await.map_err(storage_error)?{let id:String=row.get(0).map_err(storage_error)?;if seen.insert(id.clone()){frontier.push_back(id);}}
            drop(seeds);
            while seen.len()<limit {
                let Some(target)=frontier.pop_front()else{break;};
                let mut parents=c.query("SELECT edge.source FROM event_edges edge JOIN event_nodes node ON node.id=edge.source WHERE edge.target=?1 AND node.workspace_id=?2 LIMIT ?3",params![target,ws.to_string(),(limit-seen.len())as i64]).await.map_err(storage_error)?;
                while let Some(row)=parents.next().await.map_err(storage_error)?{let id:String=row.get(0).map_err(storage_error)?;if seen.insert(id.clone()){frontier.push_back(id);}}
            }
            if seen.is_empty(){return Ok(Vec::new());}
            let placeholders=(2..=seen.len()+1).map(|i|format!("?{i}")).collect::<Vec<_>>().join(",");
            let mut values=vec![ws.to_string()];values.extend(seen);
            let sql=format!("SELECT id,workspace_id,operation_id,type,summary,X'',created_at FROM events WHERE workspace_id=?1 AND id IN ({placeholders}) ORDER BY seq DESC");
            let mut rows=c.query(&sql,values).await.map_err(storage_error)?;
            let mut events=Vec::new();while let Some(row)=rows.next().await.map_err(storage_error)?{events.push(event_row(&row)?);}Ok(events)
        }).await
    }
    async fn append(&self, event: &Event, parent: Option<EventId>, kind: EdgeKind) -> Result<()> {
        let event = event.clone();
        self.run(move |c| async move {
            let tx = c
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await
                .map_err(storage_error)?;
            add_event(&tx, &event, parent, kind).await?;
            tx.commit().await.map_err(storage_error)
        })
        .await
    }
}
#[derive(Serialize, Deserialize)]
struct SnapshotDto {
    path: String,
    content: Option<Vec<u8>>,
}
#[async_trait]
impl CheckpointRepository for LocalStore {
    async fn save_checkpoint(&self, cp: &Checkpoint) -> Result<()> {
        let cp = cp.clone();
        let snapshots = cp.files.clone();
        let payload = tokio::task::spawn_blocking(move || {
            serde_json::to_vec(
                &snapshots
                    .into_iter()
                    .map(|s| SnapshotDto {
                        path: s.path,
                        content: s.content,
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(storage_error)
        })
        .await
        .map_err(storage_error)??;
        self.run(move |c| async move {
            let now = runtime_protocol::now_ms() as i64;
            c.execute(
                "INSERT INTO checkpoints VALUES(?1,?2,?3,'prepared',?4,?5,?5)",
                params![
                    cp.id.to_string(),
                    cp.workspace_id.to_string(),
                    cp.operation_id.to_string(),
                    payload,
                    now
                ],
            )
            .await
            .map_err(storage_error)?;
            Ok(())
        })
        .await
    }
    async fn checkpoint(&self, id: CheckpointId) -> Result<Checkpoint> {
        self.run(move |c| async move {
            let mut rows = c
                .query(
                    "SELECT workspace_id,operation_id,status,payload FROM checkpoints WHERE id=?1",
                    [id.to_string()],
                )
                .await
                .map_err(storage_error)?;
            checkpoint_row(
                id,
                &rows
                    .next()
                    .await
                    .map_err(storage_error)?
                    .ok_or(PortError::NotFound)?,
            )
        })
        .await
    }
    async fn complete_checkpoint(&self, id: CheckpointId) -> Result<()> {
        self.run(move |c| async move {
            c.execute(
                "UPDATE checkpoints SET status='committed',updated_at=?2 WHERE id=?1",
                params![id.to_string(), runtime_protocol::now_ms() as i64],
            )
            .await
            .map_err(storage_error)?;
            Ok(())
        })
        .await
    }
    async fn pending_checkpoints(&self, ws: WorkspaceId) -> Result<Vec<Checkpoint>> {
        self.run(move |c| async move {
            let mut rows = c.query("SELECT id,workspace_id,operation_id,status,payload FROM checkpoints WHERE workspace_id=?1 AND status='prepared'", [ws.to_string()]).await.map_err(storage_error)?;
            let mut checkpoints = Vec::new();
            while let Some(r) = rows.next().await.map_err(storage_error)? {
                let id = r.get::<String>(0).map_err(storage_error)?.parse().map_err(storage_error)?;
                let snapshots: Vec<SnapshotDto> = serde_json::from_slice(&r.get::<Vec<u8>>(4).map_err(storage_error)?).map_err(storage_error)?;
                checkpoints.push(Checkpoint { id,workspace_id:ws,operation_id:r.get::<String>(2).map_err(storage_error)?.parse().map_err(storage_error)?,files:snapshots.into_iter().map(|s| FileSnapshot {path:s.path,content:s.content}).collect(),committed:false });
            } Ok(checkpoints)
        }).await
    }
}
fn checkpoint_row(id: CheckpointId, r: &libsql::Row) -> Result<Checkpoint> {
    let snapshots: Vec<SnapshotDto> =
        serde_json::from_slice(&r.get::<Vec<u8>>(3).map_err(storage_error)?)
            .map_err(storage_error)?;
    Ok(Checkpoint {
        id,
        workspace_id: r
            .get::<String>(0)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        operation_id: r
            .get::<String>(1)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        committed: r.get::<String>(2).map_err(storage_error)? == "committed",
        files: snapshots
            .into_iter()
            .map(|s| FileSnapshot {
                path: s.path,
                content: s.content,
            })
            .collect(),
    })
}
