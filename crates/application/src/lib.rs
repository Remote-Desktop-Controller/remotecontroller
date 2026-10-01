//! Use cases depend on ports. They never access the filesystem or concrete storage.
use futures_util::FutureExt;
use runtime_domain::*;
use runtime_ports::*;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct SchedulerLimits {
    pub queue: usize,
    pub file_reads: usize,
    pub file_writes: usize,
    pub processes: usize,
    pub database: usize,
    pub cpu: usize,
    pub background: usize,
}
impl Default for SchedulerLimits {
    fn default() -> Self {
        Self {
            queue: 128,
            file_reads: 16,
            file_writes: 2,
            processes: 4,
            database: 4,
            cpu: 2,
            background: 2,
        }
    }
}
#[derive(Default)]
pub struct Metrics {
    pub queue_depth: AtomicU64,
    pub active: AtomicU64,
    pub success: AtomicU64,
    pub failure: AtomicU64,
    pub cancelled: AtomicU64,
    pub duration_ms: AtomicU64,
}
pub struct Registry {
    tools: HashMap<String, Arc<dyn Tool>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
impl Registry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let name = tool.metadata().name;
        if self.tools.contains_key(&name) {
            return Err(PortError::Conflict);
        }
        self.tools.insert(name, tool);
        Ok(())
    }
    pub fn metadata(&self) -> Vec<ToolMetadata> {
        let mut tools: Vec<_> = self.tools.values().map(|t| t.metadata()).collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }
    pub fn get(&self, name: &str) -> Result<Arc<dyn Tool>> {
        self.tools.get(name).cloned().ok_or(PortError::NotFound)
    }
}
pub struct Executor<S: Storage> {
    store: Arc<S>,
    registry: Registry,
    policy: ExecutionPolicy,
    queue: Arc<Semaphore>,
    classes: HashMap<Workload, Arc<Semaphore>>,
    cancellations: RwLock<HashMap<OperationId, CancellationToken>>,
    pub metrics: Metrics,
}
impl<S: Storage + 'static> Executor<S> {
    pub fn new(
        store: Arc<S>,
        registry: Registry,
        policy: ExecutionPolicy,
        limits: SchedulerLimits,
    ) -> Result<Arc<Self>> {
        let values = [
            (Workload::FileRead, limits.file_reads),
            (Workload::FileWrite, limits.file_writes),
            (Workload::Process, limits.processes),
            (Workload::Database, limits.database),
            (Workload::Cpu, limits.cpu),
            (Workload::Background, limits.background),
        ];
        if limits.queue == 0 || values.iter().any(|(_, n)| *n == 0) {
            return Err(PortError::Policy(
                "scheduler limits must be positive".into(),
            ));
        }
        Ok(Arc::new(Self {
            store,
            registry,
            policy,
            queue: Arc::new(Semaphore::new(limits.queue)),
            classes: values
                .into_iter()
                .map(|(c, n)| (c, Arc::new(Semaphore::new(n))))
                .collect(),
            cancellations: RwLock::new(HashMap::new()),
            metrics: Metrics::default(),
        }))
    }
    pub fn catalogue(&self) -> Vec<ToolMetadata> {
        self.registry.metadata()
    }
    pub fn capabilities(&self) -> Vec<Capability> {
        self.policy.capabilities().collect()
    }
    pub async fn recover(&self, workspace: WorkspaceId) -> Result<usize> {
        let interrupted: Vec<_> = self
            .store
            .interrupted()
            .await?
            .into_iter()
            .filter(|op| op.workspace_id == workspace)
            .collect();
        for op in &interrupted {
            self.store.finish(op.id, OperationStatus::Failed, &ToolResult { output:b"null".to_vec(),error:Some("interrupted by daemon restart; checkpoints reconciled before admission".into()) }).await?;
        }
        Ok(interrupted.len())
    }
    pub async fn get(&self, workspace: WorkspaceId, id: OperationId) -> Result<Operation> {
        let op = self.store.operation(id).await?;
        if op.workspace_id != workspace {
            return Err(PortError::NotFound);
        }
        Ok(op)
    }
    pub async fn cancel(&self, workspace: WorkspaceId, id: OperationId) -> Result<()> {
        let op = self.get(workspace, id).await?;
        if op.status.terminal() {
            return Ok(());
        }
        let tokens = self.cancellations.read().await;
        tokens.get(&id).ok_or(PortError::NotFound)?.cancel();
        Ok(())
    }
    pub async fn execute(
        self: &Arc<Self>,
        op: Operation,
        deadline: u64,
        clock: Arc<dyn ClockPort>,
        progress: Arc<dyn ProgressPort>,
    ) -> Result<Operation> {
        self.store.get_workspace(op.workspace_id).await?;
        match self.store.operation(op.id).await {
            Ok(previous) => {
                self.validate_retry(&op, &previous)?;
                return self.wait(op.workspace_id, op.id, deadline, &*clock).await;
            }
            Err(PortError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let tool = self.registry.get(&op.tool)?;
        PermissionPort::authorize(&self.policy, &tool.metadata().capabilities)?;
        let permit = self
            .queue
            .clone()
            .try_acquire_owned()
            .map_err(|_| PortError::Busy)?;
        if let Some(task_id) = op.task_id {
            self.store
                .save_task(&Task {
                    id: task_id,
                    workspace_id: op.workspace_id,
                    label: "MCP task".into(),
                })
                .await?;
        }
        if !self.store.claim(&op).await? {
            drop(permit);
            self.validate_retry(&op, &self.store.operation(op.id).await?)?;
            return self.wait(op.workspace_id, op.id, deadline, &*clock).await;
        }
        let cancellation = CancellationToken::new();
        self.cancellations
            .write()
            .await
            .insert(op.id, cancellation.clone());
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        let executor = self.clone();
        let id = op.id;
        let workspace = op.workspace_id;
        let worker_clock = clock.clone();
        // Detached from IPC lifetime: disconnect must never abort an atomic edit.
        tokio::spawn(async move {
            let _admission = permit;
            let result = executor
                .run(op, tool, cancellation, deadline, worker_clock, progress)
                .await;
            if let Err(error) = result {
                tracing::error!(operation=%id,error=%error,"execution persistence failure");
            }
            executor.cancellations.write().await.remove(&id);
        });
        self.wait(workspace, id, deadline.saturating_add(30_000), &*clock)
            .await
    }
    fn validate_retry(&self, requested: &Operation, previous: &Operation) -> Result<()> {
        if requested.workspace_id != previous.workspace_id
            || requested.tool != previous.tool
            || requested.input != previous.input
            || requested.task_id != previous.task_id
        {
            Err(PortError::Conflict)
        } else {
            Ok(())
        }
    }
    async fn wait(
        &self,
        ws: WorkspaceId,
        id: OperationId,
        deadline: u64,
        clock: &dyn ClockPort,
    ) -> Result<Operation> {
        loop {
            let op = self.get(ws, id).await?;
            if op.status.terminal() {
                return Ok(op);
            }
            if clock.now_ms() >= deadline {
                return Err(PortError::TimedOut);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    async fn run(
        self: &Arc<Self>,
        op: Operation,
        tool: Arc<dyn Tool>,
        token: CancellationToken,
        deadline: u64,
        clock: Arc<dyn ClockPort>,
        progress: Arc<dyn ProgressPort>,
    ) -> Result<()> {
        let duration = Duration::from_millis(deadline.saturating_sub(clock.now_ms()));
        let acquire = self.classes[&tool.metadata().workload]
            .clone()
            .acquire_owned();
        let admission = tokio::select! {
            biased;
            _ = token.cancelled() => Err(PortError::Cancelled),
            _ = tokio::time::sleep(duration) => Err(PortError::TimedOut),
            p = acquire => p.map_err(|_| PortError::Busy),
        };
        self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
        let permit = match admission {
            Ok(p) => p,
            Err(e) => return self.finish_error(op.id, e).await,
        };
        let _workload = permit;
        self.store.set_running(op.id).await?;
        let started = clock.now_ms();
        self.metrics.active.fetch_add(1, Ordering::Relaxed);
        let context = ToolContext {
            workspace_id: op.workspace_id,
            operation_id: op.id,
            trace_id: op.trace_id,
            cancellation: token.clone(),
            progress,
            control: self.clone(),
        };
        let input = serde_json::from_slice(&op.input).map_err(|e| PortError::Tool(e.to_string()));
        let result = match input {
            Err(e) => Err(e),
            Ok(input) => {
                let mut execution = Box::pin(async {
                    std::panic::AssertUnwindSafe(tool.execute(input, context))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| {
                            Err(PortError::Tool("tool panicked; operation failed".into()))
                        })
                });
                let remaining = Duration::from_millis(deadline.saturating_sub(clock.now_ms()));
                let workload = tool.metadata().workload;
                tokio::select! {
                    biased;
                    _ = token.cancelled() => { match execution.await {Ok(value) if workload==Workload::FileWrite=>Ok(value),Err(PortError::Cancelled)=>Err(PortError::Cancelled),Err(error) if workload==Workload::FileWrite=>Err(error),_=>Err(PortError::Cancelled)} },
                    _ = tokio::time::sleep(remaining) => { token.cancel(); match execution.await {Ok(value) if workload==Workload::FileWrite=>Ok(value),Err(PortError::Cancelled)=>Err(PortError::TimedOut),Err(error) if workload==Workload::FileWrite=>Err(error),_=>Err(PortError::TimedOut)} },
                    r = &mut execution => r,
                }
            }
        };
        self.metrics.active.fetch_sub(1, Ordering::Relaxed);
        self.metrics
            .duration_ms
            .fetch_add(clock.now_ms().saturating_sub(started), Ordering::Relaxed);
        match result {
            Ok(value) => {
                self.metrics.success.fetch_add(1, Ordering::Relaxed);
                self.store
                    .finish(
                        op.id,
                        OperationStatus::Succeeded,
                        &ToolResult {
                            output: serde_json::to_vec(&value)
                                .map_err(|e| PortError::Tool(e.to_string()))?,
                            error: None,
                        },
                    )
                    .await
            }
            Err(e) => self.finish_error(op.id, e).await,
        }
    }
    async fn finish_error(&self, id: OperationId, error: PortError) -> Result<()> {
        let status = match error {
            PortError::Cancelled => OperationStatus::Cancelled,
            PortError::TimedOut => OperationStatus::TimedOut,
            _ => OperationStatus::Failed,
        };
        if status == OperationStatus::Cancelled {
            self.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
        }
        self.metrics.failure.fetch_add(1, Ordering::Relaxed);
        self.store
            .finish(
                id,
                status,
                &ToolResult {
                    output: b"null".to_vec(),
                    error: Some(error.to_string()),
                },
            )
            .await
    }
    pub async fn shutdown(&self) {
        for token in self.cancellations.read().await.values() {
            token.cancel();
        }
        loop {
            if self.cancellations.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
#[async_trait::async_trait]
impl<S: Storage + 'static> OperationControlPort for Executor<S> {
    async fn get(&self, workspace: WorkspaceId, id: OperationId) -> Result<Operation> {
        Executor::get(self, workspace, id).await
    }
    async fn cancel(&self, workspace: WorkspaceId, id: OperationId) -> Result<()> {
        Executor::cancel(self, workspace, id).await
    }
}
