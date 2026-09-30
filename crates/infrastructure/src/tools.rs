use crate::{
    analysis::CodeAnalysis,
    context::ContextEngine,
    files::WorkspaceFiles,
    processes::ProcessManager,
    storage::{LocalStore, storage_error},
};
use async_trait::async_trait;
use runtime_application::Registry;
use runtime_domain::*;
use runtime_ports::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Services {
    pub files: WorkspaceFiles,
    pub store: Arc<LocalStore>,
    pub context: Arc<ContextEngine<LocalStore>>,
    pub processes: Arc<ProcessManager>,
    pub analysis: CodeAnalysis,
    poisoned: AtomicBool,
}
impl Services {
    pub fn new(
        files: WorkspaceFiles,
        store: Arc<LocalStore>,
        context: Arc<ContextEngine<LocalStore>>,
        processes: Arc<ProcessManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            analysis: CodeAnalysis::new(files.clone()),
            files,
            store,
            context,
            processes,
            poisoned: AtomicBool::new(false),
        })
    }
    pub fn registry(self: &Arc<Self>) -> Result<Registry> {
        let mut registry = Registry::new();
        for name in [
            "filesystem.read",
            "filesystem.read_range",
            "filesystem.list",
            "filesystem.search",
            "filesystem.stat",
            "filesystem.write",
            "filesystem.patch",
            "filesystem.batch_edit",
            "process.spawn",
            "process.status",
            "process.stdout",
            "process.stderr",
            "process.kill",
            "git.status",
            "git.diff",
            "git.show",
            "workspace.info",
            "context.recent",
            "context.causal",
            "context.compile",
            "context.event",
            "operation.status",
            "operation.cancel",
            "checkpoint.create",
            "checkpoint.rollback",
            "code.inspect",
            "code.symbols",
        ] {
            registry.register(Arc::new(Builtin {
                name: name.into(),
                services: self.clone(),
            }))?;
        }
        Ok(registry)
    }
    pub async fn recover(&self, workspace: WorkspaceId) -> Result<usize> {
        let cps = self.store.pending_checkpoints(workspace).await?;
        for cp in &cps {
            self.files.restore(cp.files.clone()).await?;
            self.store.rollback_recorded(cp.id).await?;
        }
        self.processes.reconcile().await?;
        Ok(cps.len())
    }
    async fn snapshot(&self, paths: Vec<String>, ctx: &ToolContext) -> Result<Checkpoint> {
        if paths.is_empty() || paths.len() > self.files.limits.max_batch_files {
            return Err(PortError::Policy("checkpoint count limit".into()));
        }
        let token = ctx.cancellation.clone();
        let files = self
            .files
            .blocking(move |fs| {
                let mut seen = HashSet::new();
                let mut total = 0;
                let mut files = Vec::new();
                for path in paths {
                    if token.is_cancelled() {
                        return Err(PortError::Cancelled);
                    }
                    let relative = fs.validate(&path)?;
                    let normalized = relative.to_string_lossy().to_uppercase();
                    if !seen.insert(normalized) {
                        return Err(PortError::Tool("duplicate batch path".into()));
                    }
                    let content = match fs.read_sync(&path) {
                        Ok(bytes) => Some(bytes),
                        Err(PortError::NotFound) => None,
                        Err(e) => return Err(e),
                    };
                    total += content.as_ref().map_or(0, Vec::len);
                    if total > fs.limits.max_batch_bytes {
                        return Err(PortError::Policy("checkpoint byte limit".into()));
                    }
                    files.push(FileSnapshot { path, content });
                }
                Ok(files)
            })
            .await?;
        Ok(Checkpoint {
            id: CheckpointId::new(),
            workspace_id: ctx.workspace_id,
            operation_id: ctx.operation_id,
            files,
            committed: false,
        })
    }
    async fn apply(&self, edits: Vec<Edit>, ctx: &ToolContext) -> Result<Value> {
        self.apply_bytes(
            edits
                .into_iter()
                .map(|e| ByteEdit {
                    path: e.path,
                    content: e.content.map(String::into_bytes),
                    expected_sha256: e.expected_sha256,
                })
                .collect(),
            ctx,
        )
        .await
    }
    async fn apply_bytes(&self, edits: Vec<ByteEdit>, ctx: &ToolContext) -> Result<Value> {
        let _guard = self.files.gate.write().await;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(PortError::Policy("workspace requires recovery".into()));
        }
        let size: usize = edits
            .iter()
            .map(|e| e.content.as_ref().map_or(0, Vec::len))
            .sum();
        if size > self.files.limits.max_batch_bytes {
            return Err(PortError::Policy("batch byte limit".into()));
        }
        let cp = self
            .snapshot(edits.iter().map(|e| e.path.clone()).collect(), ctx)
            .await?;
        // Prepare every path and expected content before the first mutation.
        for (edit, previous) in edits.iter().zip(&cp.files) {
            if let Some(expected) = &edit.expected_sha256 {
                let hash = previous
                    .content
                    .as_ref()
                    .map(|b| format!("{:x}", Sha256::digest(b)));
                if hash.as_deref() != Some(expected) {
                    return Err(PortError::Conflict);
                }
            }
            if edit
                .content
                .as_ref()
                .is_some_and(|s| s.len() > self.files.limits.max_file_bytes)
            {
                return Err(PortError::Policy("file size limit".into()));
            }
            if edit
                .path
                .replace('\\', "/")
                .split('/')
                .next_back()
                .is_some_and(|name| name.to_uppercase() == ".GITIGNORE")
            {
                return Err(PortError::Policy(
                    "ignore policy is immutable to tools".into(),
                ));
            }
        }
        self.store.save_checkpoint(&cp).await?;
        let token = ctx.cancellation.clone();
        let progress = ctx.progress.clone();
        let changes:Vec<_>=edits.iter().zip(&cp.files).map(|(e,old)|json!({"path":e.path,"before_bytes":old.content.as_ref().map_or(0,Vec::len),"after_bytes":e.content.as_ref().map_or(0,Vec::len)})).collect();
        let total = edits.len();
        let outcome = self
            .files
            .blocking(move |fs| {
                for (i, edit) in edits.iter().enumerate() {
                    if token.is_cancelled() {
                        return Err(PortError::Cancelled);
                    }
                    match &edit.content {
                        Some(bytes) => fs.write_sync(&edit.path, bytes)?,
                        None => fs.restore_sync(&[FileSnapshot {
                            path: edit.path.clone(),
                            content: None,
                        }])?,
                    }
                    match &edit.content {
                        Some(bytes) if fs.read_sync(&edit.path)? != *bytes => {
                            return Err(PortError::Tool("post-write verification failed".into()));
                        }
                        None if !matches!(fs.read_sync(&edit.path), Err(PortError::NotFound)) => {
                            return Err(PortError::Tool("delete verification failed".into()));
                        }
                        _ => {}
                    }
                    if i % 16 == 0 || i + 1 == total {
                        progress.emit(i + 1, total, "batch applied");
                    }
                }
                if token.is_cancelled() {
                    return Err(PortError::Cancelled);
                }
                Ok(())
            })
            .await;
        if let Err(error) = outcome {
            let originals = cp.files.clone();
            if let Err(restore) = self
                .files
                .blocking(move |fs| fs.restore_sync(&originals))
                .await
            {
                self.poisoned.store(true, Ordering::Release);
                return Err(PortError::Tool(format!(
                    "{error}; rollback failed: {restore}; workspace requires recovery"
                )));
            }
            self.store.rollback_recorded(cp.id).await?;
            self.store
                .append(
                    &Event {
                        id: EventId::new(),
                        workspace_id: ctx.workspace_id,
                        operation_id: ctx.operation_id,
                        kind: "rollback".into(),
                        summary: format!("checkpoint {} restored after {error}", cp.id),
                        raw: b"null".to_vec(),
                        created_at: runtime_protocol::now_ms(),
                    },
                    None,
                    EdgeKind::RolledBackFrom,
                )
                .await?;
            return Err(error);
        }
        // Artifact rows + mutation event + journal commit share one DB transaction.
        let workspace = ctx.workspace_id;
        let operation = ctx.operation_id;
        let checkpoint = cp.id;
        let paths: Vec<_> = cp.files.iter().map(|s| s.path.clone()).collect();
        let count = paths.len();
        let now = runtime_protocol::now_ms();
        let commit_result=self.store.run(move|c|async move{
            let tx=c.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await.map_err(storage_error)?;
            for path in &paths{
                tx.execute("INSERT INTO artifacts VALUES(?1,?2,?3,?4,'file_mutation',NULL,?5)",libsql::params![ArtifactId::new().to_string(),workspace.to_string(),operation.to_string(),path.clone(),now as i64]).await.map_err(storage_error)?;
            }
            let event=EventId::new();
            let summary=format!("{count} files changed; checkpoint={checkpoint}; paths={}",paths.iter().take(20).cloned().collect::<Vec<_>>().join(","));
            tx.execute("INSERT INTO events(id,workspace_id,operation_id,type,summary,payload,created_at) VALUES(?1,?2,?3,'mutated',?4,?5,?6)",libsql::params![event.to_string(),workspace.to_string(),operation.to_string(),summary,serde_json::to_vec(&paths).map_err(storage_error)?,now as i64]).await.map_err(storage_error)?;
            tx.execute("INSERT INTO event_nodes VALUES(?1,?2,?3)",libsql::params![event.to_string(),workspace.to_string(),operation.to_string()]).await.map_err(storage_error)?;
            tx.execute("INSERT INTO event_edges SELECT id,?2,'Mutated' FROM events WHERE operation_id=?1 AND type='queued'",libsql::params![operation.to_string(),event.to_string()]).await.map_err(storage_error)?;
            tx.execute("UPDATE workspaces SET graph_version=graph_version+1 WHERE id=?1",[workspace.to_string()]).await.map_err(storage_error)?;
            tx.execute("UPDATE checkpoints SET status='committed',updated_at=?2 WHERE id=?1",libsql::params![checkpoint.to_string(),now as i64]).await.map_err(storage_error)?;
            tx.commit().await.map_err(storage_error)
        }).await;
        if let Err(error) = commit_result {
            let originals = cp.files.clone();
            if self
                .files
                .blocking(move |fs| fs.restore_sync(&originals))
                .await
                .is_err()
                || self.store.rollback_recorded(cp.id).await.is_err()
            {
                self.poisoned.store(true, Ordering::Release);
            }
            return Err(error);
        }
        Ok(
            json!({"checkpoint_id":cp.id.to_string(),"changed":count,"diff":changes.into_iter().take(1000).collect::<Vec<_>>(),"diff_truncated":count>1000}),
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub path: String,
    pub content: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}
struct ByteEdit {
    path: String,
    content: Option<Vec<u8>>,
    expected_sha256: Option<String>,
}
struct Builtin {
    name: String,
    services: Arc<Services>,
}
fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PortError::Tool(format!("required string: {key}")))
}
fn id<T: std::str::FromStr>(input: &Value, key: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    text(input, key)?
        .parse()
        .map_err(|e| PortError::Tool(format!("{e}")))
}
pub fn operation_json(op: &Operation) -> Value {
    json!({"operation_id":op.id.to_string(),"workspace_id":op.workspace_id.to_string(),"task_id":op.task_id.map(|id|id.to_string()),"trace_id":op.trace_id.to_string(),"tool":op.tool,"status":format!("{:?}",op.status),"result":op.result.as_ref().and_then(|r|serde_json::from_slice::<Value>(&r.output).ok()),"error":op.result.as_ref().and_then(|r|r.error.clone())})
}
#[async_trait]
impl Tool for Builtin {
    fn metadata(&self) -> ToolMetadata {
        let name = self.name.as_str();
        let (capabilities, workload, risk) = if name.starts_with("filesystem.")
            && matches!(
                name,
                "filesystem.write" | "filesystem.patch" | "filesystem.batch_edit"
            )
            || name.starts_with("checkpoint.")
        {
            (
                vec![Capability::WriteWorkspace],
                Workload::FileWrite,
                "write",
            )
        } else if name == "process.spawn" {
            (
                vec![
                    Capability::SpawnProcess,
                    Capability::OutsideWorkspaceAccess,
                    Capability::NetworkAccess,
                ],
                Workload::Process,
                "high",
            )
        } else if name == "process.kill" {
            (vec![Capability::KillProcess], Workload::Database, "high")
        } else if name.starts_with("git.") {
            (vec![Capability::GitRead], Workload::Process, "read")
        } else if name.starts_with("code.") {
            (vec![Capability::ReadWorkspace], Workload::Cpu, "read")
        } else {
            (
                vec![Capability::ReadWorkspace],
                if name.starts_with("filesystem.") {
                    Workload::FileRead
                } else {
                    Workload::Database
                },
                "read",
            )
        };
        let (properties, required) = match name {
            "filesystem.read" | "filesystem.stat" => {
                (json!({"path":{"type":"string"}}), vec!["path"])
            }
            "filesystem.read_range" => (
                json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}}),
                vec!["path"],
            ),
            "filesystem.list" | "filesystem.search" => (
                json!({"path":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100000},"query":{"type":"string"},"content":{"type":"boolean"}}),
                vec![],
            ),
            "filesystem.write" => (
                json!({"path":{"type":"string"},"content":{"type":"string"},"expected_sha256":{"type":"string"}}),
                vec!["path", "content"],
            ),
            "filesystem.patch" => (
                json!({"path":{"type":"string"},"find":{"type":"string"},"replace":{"type":"string"}}),
                vec!["path", "find", "replace"],
            ),
            "filesystem.batch_edit" => (
                json!({"edits":{"type":"array","minItems":1,"maxItems":10000,"items":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":["string","null"]},"expected_sha256":{"type":["string","null"]}},"required":["path","content"],"additionalProperties":false}}}),
                vec!["edits"],
            ),
            "process.spawn" => (
                json!({"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"}}),
                vec!["program", "args"],
            ),
            "process.status" | "process.stdout" | "process.stderr" | "process.kill" => (
                json!({"process_id":{"type":"string","format":"uuid"}}),
                vec!["process_id"],
            ),
            "context.event" => (
                json!({"event_id":{"type":"string","format":"uuid"}}),
                vec!["event_id"],
            ),
            "operation.status" | "operation.cancel" => (
                json!({"operation_id":{"type":"string","format":"uuid"}}),
                vec!["operation_id"],
            ),
            "context.causal" | "context.compile" => (
                json!({"operation_id":{"type":"string","format":"uuid"},"budget_bytes":{"type":"integer","minimum":128,"maximum":131072}}),
                vec![],
            ),
            "context.recent" => (
                json!({"limit":{"type":"integer","minimum":1,"maximum":1000}}),
                vec![],
            ),
            "checkpoint.create" => (
                json!({"paths":{"type":"array","items":{"type":"string"}}}),
                vec!["paths"],
            ),
            "checkpoint.rollback" => (
                json!({"checkpoint_id":{"type":"string","format":"uuid"}}),
                vec!["checkpoint_id"],
            ),
            "code.inspect" | "code.symbols" => (
                json!({"path":{"type":"string"},"language":{"type":"string","enum":["rust","python"]},"query":{"type":"string"}}),
                vec!["path", "language"],
            ),
            "git.show" => (json!({"revision":{"type":"string"}}), vec!["revision"]),
            _ => (json!({}), vec![]),
        };
        ToolMetadata {
            name: self.name.clone(),
            version: "1.0.0".into(),
            description: format!(
                "Local workspace tool: {}. Requires {:?}.",
                self.name, capabilities
            ),
            capabilities,
            input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            output_schema: json!({"type":"object"}),
            risk: risk.into(),
            workload,
        }
    }
    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<Value> {
        if self.services.poisoned.load(Ordering::Acquire) {
            return Err(PortError::Policy("workspace requires recovery".into()));
        }
        if ctx.cancellation.is_cancelled() {
            return Err(PortError::Cancelled);
        }
        let s = &self.services;
        match self.name.as_str() {
            "workspace.info" => {
                let ws = s.store.get_workspace(ctx.workspace_id).await?;
                Ok(
                    json!({"workspace_id":ws.id.to_string(),"root":ws.root,"graph_version":ws.graph_version}),
                )
            }
            "filesystem.read" | "filesystem.read_range" => {
                let path = text(&input, "path")?;
                let bytes = s.files.read(path).await?;
                let content = String::from_utf8(bytes)
                    .map_err(|_| PortError::Tool("file is not UTF-8".into()))?;
                let content = if self.name == "filesystem.read_range" {
                    let start = input["start_line"].as_u64().unwrap_or(1) as usize;
                    let end = input["end_line"].as_u64().unwrap_or(start as u64 + 100) as usize;
                    if start == 0 || end < start {
                        return Err(PortError::Tool("invalid line range".into()));
                    }
                    content
                        .lines()
                        .skip(start - 1)
                        .take(end - start + 1)
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    content
                };
                Ok(json!({"path":path,"content":content}))
            }
            "filesystem.stat" => s.files.stat(text(&input, "path")?).await,
            "filesystem.list" | "filesystem.search" => {
                let limit = input["limit"].as_u64().unwrap_or(1000) as usize;
                let paths = s
                    .files
                    .scan(
                        input["path"].as_str().unwrap_or("."),
                        s.files.limits.max_scan_entries,
                        ctx.cancellation.clone(),
                    )
                    .await?;
                let query = input["query"].as_str().unwrap_or("");
                let content = input["content"].as_bool().unwrap_or(false);
                let mut found = Vec::new();
                for path in paths {
                    if ctx.cancellation.is_cancelled() {
                        return Err(PortError::Cancelled);
                    }
                    let matches = path.contains(query)
                        || (content
                            && s.files
                                .read(&path)
                                .await
                                .is_ok_and(|b| String::from_utf8_lossy(&b).contains(query)));
                    if self.name == "filesystem.list" || matches {
                        found.push(path);
                        if found.len() >= limit {
                            break;
                        }
                    }
                }
                Ok(json!({"paths":found,"limit":limit,"truncated":found.len()>=limit}))
            }
            "filesystem.write" => {
                s.apply(
                    vec![Edit {
                        path: text(&input, "path")?.into(),
                        content: Some(text(&input, "content")?.into()),
                        expected_sha256: input["expected_sha256"].as_str().map(String::from),
                    }],
                    &ctx,
                )
                .await
            }
            "filesystem.patch" => {
                let path = text(&input, "path")?;
                let old = s.files.read(path).await?;
                let hash = format!("{:x}", Sha256::digest(&old));
                let old = String::from_utf8(old).map_err(|e| PortError::Tool(e.to_string()))?;
                let find = text(&input, "find")?;
                if find.is_empty() || old.matches(find).count() != 1 {
                    return Err(PortError::Conflict);
                }
                s.apply(
                    vec![Edit {
                        path: path.into(),
                        content: Some(old.replacen(find, text(&input, "replace")?, 1)),
                        expected_sha256: Some(hash),
                    }],
                    &ctx,
                )
                .await
            }
            "filesystem.batch_edit" => {
                let edits: Vec<Edit> = serde_json::from_value(input["edits"].clone())
                    .map_err(|e| PortError::Tool(e.to_string()))?;
                s.apply(edits, &ctx).await
            }
            "checkpoint.create" => {
                let _guard = s.files.gate.write().await;
                let paths = serde_json::from_value(input["paths"].clone())
                    .map_err(|e| PortError::Tool(e.to_string()))?;
                let cp = s.snapshot(paths, &ctx).await?;
                s.store.save_checkpoint(&cp).await?;
                s.store.complete_checkpoint(cp.id).await?;
                Ok(json!({"checkpoint_id":cp.id.to_string(),"files":cp.files.len()}))
            }
            "checkpoint.rollback" => {
                let cp = s.store.checkpoint(id(&input, "checkpoint_id")?).await?;
                if cp.workspace_id != ctx.workspace_id {
                    return Err(PortError::NotFound);
                }
                let edits = cp
                    .files
                    .into_iter()
                    .map(|f| ByteEdit {
                        path: f.path,
                        content: f.content,
                        expected_sha256: None,
                    })
                    .collect::<Vec<_>>();
                let result = s.apply_bytes(edits, &ctx).await?;
                s.store.rollback_recorded(cp.id).await?;
                Ok(result)
            }
            "process.spawn" => {
                let args = serde_json::from_value(input["args"].clone())
                    .map_err(|e| PortError::Tool(e.to_string()))?;
                s.processes
                    .spawn(
                        ProcessSpec {
                            program: text(&input, "program")?.into(),
                            args,
                            cwd: input["cwd"].as_str().unwrap_or(".").into(),
                        },
                        ctx,
                    )
                    .await
            }
            "process.status" | "process.stdout" | "process.stderr" => {
                s.processes
                    .status(
                        ctx.workspace_id,
                        id(&input, "process_id")?,
                        match self.name.as_str() {
                            "process.stdout" => Some(false),
                            "process.stderr" => Some(true),
                            _ => None,
                        },
                    )
                    .await
            }
            "process.kill" => {
                s.processes
                    .kill(ctx.workspace_id, id(&input, "process_id")?)
                    .await?;
                Ok(json!({"cancel_requested":true}))
            }
            "operation.status" => Ok(operation_json(
                &ctx.control
                    .get(ctx.workspace_id, id(&input, "operation_id")?)
                    .await?,
            )),
            "operation.cancel" => {
                ctx.control
                    .cancel(ctx.workspace_id, id(&input, "operation_id")?)
                    .await?;
                Ok(json!({"cancel_requested":true}))
            }
            "context.event" => {
                let event = s
                    .store
                    .raw_event(ctx.workspace_id, id(&input, "event_id")?)
                    .await?;
                Ok(
                    json!({"event_id":event.id.to_string(),"operation_id":event.operation_id.to_string(),"summary":event.summary,"payload":serde_json::from_slice::<Value>(&event.raw).map_err(|e|PortError::Storage(e.to_string()))?}),
                )
            }
            "context.recent" => {
                let events = s
                    .store
                    .recent(
                        ctx.workspace_id,
                        input["limit"].as_u64().unwrap_or(20) as usize,
                    )
                    .await?;
                Ok(
                    json!({"events":events.into_iter().map(|e|json!({"event_id":e.id.to_string(),"operation_id":e.operation_id.to_string(),"type":e.kind,"summary":e.summary})).collect::<Vec<_>>()}),
                )
            }
            "context.causal" | "context.compile" => {
                let op = input["operation_id"]
                    .as_str()
                    .map(str::parse)
                    .transpose()
                    .map_err(|e| PortError::Tool(format!("{e}")))?;
                let budget = input["budget_bytes"].as_u64().unwrap_or(8192) as usize;
                Ok(json!({"context":s.context.compile(ctx.workspace_id,op,budget).await?}))
            }
            "code.inspect" | "code.symbols" => {
                s.analysis
                    .inspect(
                        text(&input, "path")?,
                        text(&input, "language")?,
                        input["query"].as_str(),
                    )
                    .await
            }
            "git.status" | "git.diff" | "git.show" => {
                crate::git::GitReader::new(s.files.clone())
                    .read_git(self.name.as_str(), input["revision"].as_str())
                    .await
            }
            _ => Err(PortError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn poisoned_workspace_is_checked_under_mutation_lock() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let k = crate::kernel::Kernel::open(
            root.path(),
            state.path(),
            crate::config::RuntimeConfig::default(),
            ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]),
        )
        .await
        .unwrap();
        let op = Operation {
            id: OperationId::new(),
            workspace_id: k.workspace.id,
            task_id: None,
            trace_id: TraceId::new(),
            tool: "filesystem.write".into(),
            input: b"{}".to_vec(),
            status: OperationStatus::Queued,
            result: None,
            created_at: runtime_protocol::now_ms(),
        };
        k.services.store.claim(&op).await.unwrap();
        k.services.poisoned.store(true, Ordering::Release);
        let ctx = ToolContext {
            workspace_id: op.workspace_id,
            operation_id: op.id,
            trace_id: op.trace_id,
            cancellation: tokio_util::sync::CancellationToken::new(),
            progress: Arc::new(NoProgress),
            control: k.executor.clone(),
        };
        let result = k
            .services
            .apply_bytes(
                vec![ByteEdit {
                    path: "must-not-write".into(),
                    content: Some(vec![1]),
                    expected_sha256: None,
                }],
                &ctx,
            )
            .await;
        assert!(result.is_err());
        assert!(!root.path().join("must-not-write").exists());
    }
}
