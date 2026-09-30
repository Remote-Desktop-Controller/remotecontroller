//! Application-facing contracts; no concrete database, cache or OS implementation.
use async_trait::async_trait;
use runtime_domain::*;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("policy: {0}")]
    Policy(String),
    #[error("tool: {0}")]
    Tool(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("operation timed out")]
    TimedOut,
    #[error("queue full")]
    Busy,
    #[error("not found")]
    NotFound,
    #[error("operation id conflicts with a different request")]
    Conflict,
}
pub type Result<T> = std::result::Result<T, PortError>;

#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn open_workspace(&self, root: &str) -> Result<Workspace>;
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace>;
}
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn save_task(&self, task: &Task) -> Result<()>;
}
#[async_trait]
pub trait OperationRepository: Send + Sync {
    /// Atomically insert a queued operation. False means the ID already exists.
    async fn claim(&self, op: &Operation) -> Result<bool>;
    async fn operation(&self, id: OperationId) -> Result<Operation>;
    async fn set_running(&self, id: OperationId) -> Result<()>;
    /// Commit result, final event, graph edges and graph version in one transaction.
    async fn finish(
        &self,
        id: OperationId,
        status: OperationStatus,
        result: &ToolResult,
    ) -> Result<()>;
    async fn interrupted(&self) -> Result<Vec<Operation>>;
}
#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn raw_event(&self, workspace: WorkspaceId, id: EventId) -> Result<Event>;
    async fn recent(&self, workspace: WorkspaceId, limit: usize) -> Result<Vec<Event>>;
    async fn causal(
        &self,
        workspace: WorkspaceId,
        operation: OperationId,
        limit: usize,
    ) -> Result<Vec<Event>>;
    async fn append(&self, event: &Event, parent: Option<EventId>, kind: EdgeKind) -> Result<()>;
}
#[async_trait]
pub trait CheckpointRepository: Send + Sync {
    async fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()>;
    async fn checkpoint(&self, id: CheckpointId) -> Result<Checkpoint>;
    async fn complete_checkpoint(&self, id: CheckpointId) -> Result<()>;
    async fn pending_checkpoints(&self, workspace: WorkspaceId) -> Result<Vec<Checkpoint>>;
}
pub trait Storage:
    WorkspaceRepository + TaskRepository + OperationRepository + EventRepository + CheckpointRepository
{
}
impl<
    T: WorkspaceRepository
        + TaskRepository
        + OperationRepository
        + EventRepository
        + CheckpointRepository,
> Storage for T
{
}

#[async_trait]
pub trait ContextStoragePort: Send + Sync {
    async fn compile(
        &self,
        workspace: WorkspaceId,
        operation: Option<OperationId>,
        budget: usize,
    ) -> Result<String>;
}
#[async_trait]
pub trait CachePort: Send + Sync {
    async fn get(&self, key: &str) -> Option<String>;
    async fn put(&self, key: String, value: String);
    async fn invalidate(&self, key: &str);
}
#[async_trait]
pub trait FileSystemPort: Send + Sync {
    async fn read(&self, path: &str) -> Result<Vec<u8>>;
    async fn write_atomic(&self, path: &str, bytes: Vec<u8>) -> Result<()>;
    async fn restore(&self, snapshots: Vec<FileSnapshot>) -> Result<()>;
    async fn scan(
        &self,
        path: &str,
        limit: usize,
        cancel: CancellationToken,
    ) -> Result<Vec<String>>;
}
#[async_trait]
pub trait ProcessPort: Send + Sync {
    async fn spawn(&self, spec: ProcessSpec, context: ToolContext) -> Result<Value>;
    async fn status(
        &self,
        workspace: WorkspaceId,
        id: ProcessId,
        stream: Option<bool>,
    ) -> Result<Value>;
    async fn kill(&self, workspace: WorkspaceId, id: ProcessId) -> Result<()>;
}
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}
#[async_trait]
pub trait GitPort: Send + Sync {
    async fn read_git(&self, command: &str, argument: Option<&str>) -> Result<Value>;
}
#[async_trait]
pub trait CodeAnalysisPort: Send + Sync {
    async fn inspect(&self, path: &str, language: &str, query: Option<&str>) -> Result<Value>;
}
pub trait PermissionPort: Send + Sync {
    fn authorize(&self, capabilities: &[Capability]) -> Result<()>;
}
pub trait ClockPort: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Debug)]
pub struct ToolMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<Capability>,
    pub input_schema: Value,
    pub output_schema: Value,
    pub risk: String,
    pub workload: Workload,
}
#[derive(Clone)]
pub struct ToolContext {
    pub workspace_id: WorkspaceId,
    pub operation_id: OperationId,
    pub trace_id: TraceId,
    pub cancellation: CancellationToken,
    pub progress: Arc<dyn ProgressPort>,
    pub control: Arc<dyn OperationControlPort>,
}
#[async_trait]
pub trait OperationControlPort: Send + Sync {
    async fn get(&self, workspace: WorkspaceId, id: OperationId) -> Result<Operation>;
    async fn cancel(&self, workspace: WorkspaceId, id: OperationId) -> Result<()>;
}
pub trait ProgressPort: Send + Sync {
    fn emit(&self, completed: usize, total: usize, message: &str);
}
pub struct NoProgress;
impl ProgressPort for NoProgress {
    fn emit(&self, _: usize, _: usize, _: &str) {}
}
#[async_trait]
pub trait Tool: Send + Sync {
    fn metadata(&self) -> ToolMetadata;
    async fn execute(&self, input: Value, context: ToolContext) -> Result<Value>;
}
#[async_trait]
pub trait IpcTransportPort: Send + Sync {
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>>;
}
