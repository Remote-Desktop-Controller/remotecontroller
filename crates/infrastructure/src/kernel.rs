//! Daemon composition boundary. Gateway never links this crate.
use crate::{
    config::RuntimeConfig,
    context::ContextEngine,
    files::WorkspaceFiles,
    processes::ProcessManager,
    storage::LocalStore,
    tools::{Services, operation_json},
};
use runtime_application::Executor;
use runtime_domain::*;
use runtime_ports::*;
use runtime_protocol::*;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;
pub struct SystemClock;
impl ClockPort for SystemClock {
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}
pub struct ChannelProgress {
    pub sender: mpsc::Sender<EventEnvelope>,
    pub meta: Metadata,
}
impl ProgressPort for ChannelProgress {
    fn emit(&self, completed: usize, total: usize, message: &str) {
        let _ = self.sender.try_send(EventEnvelope {
            meta: self.meta.clone(),
            completed,
            total,
            message: message.into(),
        });
    }
}
pub struct Kernel {
    pub shutdown: tokio_util::sync::CancellationToken,
    pub workspace: Workspace,
    pub executor: Arc<Executor<LocalStore>>,
    pub services: Arc<Services>,
    pub config: RuntimeConfig,
    pub telemetry: Arc<crate::observer::TelemetryHub>,
    pub layout: Arc<crate::storage::StateLayout>,
    pub checkpoints: Arc<crate::storage::AtomicCheckpointManager>,
}
impl Kernel {
    pub async fn open(
        root: &Path,
        state: &Path,
        config: RuntimeConfig,
        policy: ExecutionPolicy,
    ) -> Result<Arc<Self>> {
        config.validate()?;
        let files = WorkspaceFiles::open(root, config.files.clone())?;
        let store = Arc::new(LocalStore::open(&state.join("runtime.db")).await?);
        let workspace = store
            .open_workspace(&files.root().to_string_lossy())
            .await?;
        let context = Arc::new(ContextEngine::new(
            store.clone(),
            config.cache_capacity,
            Duration::from_secs(config.cache_ttl_seconds),
        ));
        let processes = ProcessManager::new(
            files.clone(),
            store.clone(),
            config.allowed_programs.clone(),
            config.process_buffer_bytes,
        )?
        .with_spool_limits(config.process_spool_bytes, config.process_spool_total_bytes)?;
        #[cfg(unix)]
        let processes = if let Some(path) = config.guardian_path.clone() {
            processes.with_guardian(path)?
        } else {
            processes
        };
        let processes = Arc::new(processes);
        let services = Services::new(files, store.clone(), context, processes);
        services.recover(workspace.id).await?;
        let executor = Executor::new(
            store,
            services.registry()?,
            policy,
            config.scheduler.clone().into(),
        )?;
        executor.recover(workspace.id).await?;
        let telemetry = Arc::new(crate::observer::TelemetryHub::default());
        let shutdown = tokio_util::sync::CancellationToken::new();
        let layout = Arc::new(crate::storage::StateLayout::new(state));
        let checkpoints = Arc::new(crate::storage::AtomicCheckpointManager::new(
            (*layout).clone(),
        ));
        let _ = telemetry.attach_audit_logger((*layout).clone(), shutdown.clone());
        Ok(Arc::new(Self {
            shutdown,
            workspace,
            executor,
            services,
            config,
            telemetry,
            layout,
            checkpoints,
        }))
    }
    pub async fn handle(
        self: &Arc<Self>,
        request: RequestEnvelope,
        progress: Arc<dyn ProgressPort>,
    ) -> ResponseEnvelope {
        let meta = request.meta.clone();
        ResponseEnvelope {
            meta,
            payload: self.dispatch(request, progress).await.map_err(wire_error),
        }
    }
    /// Executes a request through the kernel: policy, checkpointing, telemetry and the
    /// tool registry all apply exactly as in the main IPC path.
    ///
    /// Public so auxiliary local ingress channels (for example the Maestro order pipe)
    /// reuse the same execution track instead of reimplementing dispatch.
    pub async fn dispatch(
        self: &Arc<Self>,
        request: RequestEnvelope,
        progress: Arc<dyn ProgressPort>,
    ) -> Result<serde_json::Value> {
        let meta = request.meta;
        if meta.protocol_version != VERSION {
            return Err(PortError::Policy("IPC version mismatch".into()));
        }
        if meta
            .workspace_id
            .is_some_and(|ws| ws != self.workspace.id.as_uuid())
        {
            return Err(PortError::NotFound);
        }
        if now_ms() >= meta.deadline {
            return Err(PortError::TimedOut);
        }
        match request.payload {
            Request::Catalog => {
                let tools: Vec<_> = self
                    .executor
                    .catalogue()
                    .into_iter()
                    .map(|t| ToolDescriptor {
                        name: t.name,
                        version: t.version,
                        description: t.description,
                        capabilities: t
                            .capabilities
                            .into_iter()
                            .map(|c| format!("{c:?}"))
                            .collect(),
                        risk: t.risk,
                        input_schema: t.input_schema,
                        output_schema: t.output_schema,
                    })
                    .collect();
                Ok(
                    serde_json::json!({"workspace_id":self.workspace.id.to_string(),"protocol_version":VERSION,"tools":tools,"operation_timeout_ms":self.config.operation_timeout_ms}),
                )
            }
            Request::Heartbeat => {
                use std::sync::atomic::Ordering::Relaxed;
                let m = &self.executor.metrics;
                let c = &self.services.context.metrics;
                let s = &self.services.store.metrics;
                use sha2::Digest;
                let config_fingerprint = format!(
                    "{:x}",
                    sha2::Sha256::digest(
                        serde_json::to_vec(&self.config)
                            .map_err(|e| PortError::Tool(e.to_string()))?
                    )
                );
                Ok(
                    serde_json::json!({"alive":true,"workspace_id":self.workspace.id.to_string(),"root":self.workspace.root,"capabilities":self.executor.capabilities().into_iter().map(|c|format!("{c:?}")).collect::<Vec<_>>(),"config_fingerprint":config_fingerprint,"metrics":{"queue_depth":m.queue_depth.load(Relaxed),"active_operations":m.active.load(Relaxed),"tool_success":m.success.load(Relaxed),"tool_failure":m.failure.load(Relaxed),"cancel_count":m.cancelled.load(Relaxed),"operation_duration_ms_total":m.duration_ms.load(Relaxed),"cache_hits":c.hits.load(Relaxed),"cache_misses":c.misses.load(Relaxed),"storage_calls":s.calls.load(Relaxed),"storage_micros_total":s.elapsed_micros.load(Relaxed)}}),
                )
            }
            Request::Shutdown => Ok(serde_json::json!({"stopping":true})),
            Request::GetOperation { operation_id } => Ok(operation_json(
                &self
                    .executor
                    .get(self.workspace.id, parse_id(operation_id)?)
                    .await?,
            )),
            Request::Cancel { operation_id } => {
                self.executor
                    .cancel(self.workspace.id, parse_id(operation_id)?)
                    .await?;
                Ok(serde_json::json!({"cancel_requested":true}))
            }
            Request::Execute { tool, input } => {
                if meta.workspace_id.is_none() {
                    return Err(PortError::Policy(
                        "workspace_id required for execution".into(),
                    ));
                }
                let metadata = self
                    .executor
                    .catalogue()
                    .into_iter()
                    .find(|m| m.name == tool)
                    .ok_or(PortError::NotFound)?;
                validate_input(&input, &metadata.input_schema)?;
                if tool == "operation.cancel" || tool == "operation.status" {
                    let target: OperationId = input["operation_id"]
                        .as_str()
                        .ok_or(PortError::NotFound)?
                        .parse()
                        .map_err(|e: DomainError| PortError::Tool(e.to_string()))?;
                    if tool == "operation.cancel" {
                        self.executor.cancel(self.workspace.id, target).await?;
                        return Ok(serde_json::json!({"cancel_requested":true}));
                    }
                    return Ok(operation_json(
                        &self.executor.get(self.workspace.id, target).await?,
                    ));
                }
                let op = Operation {
                    id: parse_id(meta.operation_id)?,
                    workspace_id: self.workspace.id,
                    task_id: meta.task_id.map(parse_id).transpose()?,
                    trace_id: parse_id(meta.trace_id)?,
                    tool: tool.clone(),
                    input: serde_json::to_vec(&input)
                        .map_err(|e| PortError::Tool(e.to_string()))?,
                    status: OperationStatus::Queued,
                    result: None,
                    created_at: now_ms(),
                };
                let deadline = meta
                    .deadline
                    .min(now_ms().saturating_add(self.config.operation_timeout_ms));

                // Pre-compute diff for all workspace file mutation tools before execution modifies disk
                let pre_diff = if tool == "filesystem.write" {
                    if let Some(path_str) = input.get("path").and_then(|v| v.as_str()) {
                        let content_bytes = input
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|s| s.as_bytes())
                            .unwrap_or_default();
                        let target_path = self.services.files.root().join(path_str);
                        let diff =
                            crate::telemetry::diff::compute_file_diff(&target_path, content_bytes);
                        Some((path_str.to_string(), diff))
                    } else {
                        None
                    }
                } else if tool == "filesystem.patch" {
                    if let (Some(path_str), Some(find), Some(replace)) = (
                        input.get("path").and_then(|v| v.as_str()),
                        input.get("find").and_then(|v| v.as_str()),
                        input.get("replace").and_then(|v| v.as_str()),
                    ) {
                        let target_path = self.services.files.root().join(path_str);
                        let old_bytes = if target_path.is_file() {
                            std::fs::read(&target_path).unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        let new_bytes = if let Ok(old_str) = std::str::from_utf8(&old_bytes) {
                            old_str.replacen(find, replace, 1).into_bytes()
                        } else {
                            Vec::new()
                        };
                        let diff =
                            crate::telemetry::diff::compute_content_diff(&old_bytes, &new_bytes);
                        Some((path_str.to_string(), diff))
                    } else {
                        None
                    }
                } else if tool == "filesystem.batch_edit" {
                    if let Some(edits) = input.get("edits").and_then(|v| v.as_array()) {
                        let mut total_added = 0;
                        let mut total_removed = 0;
                        let mut paths_summary = Vec::new();
                        for edit in edits {
                            if let Some(path_str) = edit.get("path").and_then(|v| v.as_str()) {
                                paths_summary.push(path_str.to_string());
                                let target_path = self.services.files.root().join(path_str);
                                let old_bytes = if target_path.is_file() {
                                    std::fs::read(&target_path).unwrap_or_default()
                                } else {
                                    Vec::new()
                                };
                                let content_bytes = edit
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.as_bytes())
                                    .unwrap_or_default();
                                let diff = crate::telemetry::diff::compute_content_diff(
                                    &old_bytes,
                                    content_bytes,
                                );
                                total_added += diff.lines_added;
                                total_removed += diff.lines_removed;
                            }
                        }
                        let summary_name = if paths_summary.len() == 1 {
                            paths_summary[0].clone()
                        } else {
                            format!(
                                "batch[{} files: {}]",
                                paths_summary.len(),
                                paths_summary.join(", ")
                            )
                        };
                        Some((
                            summary_name,
                            crate::telemetry::diff::DiffMetrics {
                                lines_added: total_added,
                                lines_removed: total_removed,
                                total_bytes: 0,
                                is_binary: false,
                            },
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let start_time = std::time::Instant::now();
                let exec_result = self
                    .executor
                    .execute(op, deadline, Arc::new(SystemClock), progress)
                    .await;
                let elapsed_ms = start_time.elapsed().as_millis() as u64;

                let audit_status = match &exec_result {
                    Ok(res) if res.status == OperationStatus::Succeeded => {
                        crate::observer::event::AuditStatus::Success
                    }
                    Ok(res) if res.status == OperationStatus::Cancelled => {
                        crate::observer::event::AuditStatus::Cancelled
                    }
                    _ => crate::observer::event::AuditStatus::Failed,
                };

                // Real-time interception & audit event broadcast
                if let Some((path_str, diff)) = pre_diff {
                    self.telemetry
                        .broadcast(crate::observer::event::AuditEvent::file_modified(
                            path_str,
                            diff.lines_added,
                            diff.lines_removed,
                            elapsed_ms,
                            audit_status,
                        ));
                } else if tool.starts_with("filesystem.")
                    && (tool == "filesystem.write"
                        || tool == "filesystem.patch"
                        || tool == "filesystem.batch_edit")
                {
                    let path_str = input
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    self.telemetry
                        .broadcast(crate::observer::event::AuditEvent::file_modified(
                            path_str,
                            0,
                            0,
                            elapsed_ms,
                            audit_status,
                        ));
                } else if tool == "process.spawn" || tool == "process.run" {
                    let cmd_display =
                        if let Some(prog) = input.get("program").and_then(|v| v.as_str()) {
                            let args_str = input
                                .get("args")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|x| x.as_str())
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .unwrap_or_default();
                            format!("{prog} {args_str}")
                        } else {
                            tool.clone()
                        };
                    self.telemetry
                        .broadcast(crate::observer::event::AuditEvent::command_executed(
                            &cmd_display,
                            elapsed_ms,
                            audit_status,
                            Some(input.clone()),
                        ));
                } else {
                    self.telemetry
                        .broadcast(crate::observer::event::AuditEvent::tool_invoked(
                            &tool,
                            elapsed_ms,
                            audit_status,
                            Some(input.clone()),
                        ));
                }

                Ok(operation_json(&exec_result?))
            }
        }
    }
}
fn parse_id<T: std::str::FromStr<Err = DomainError>>(id: uuid::Uuid) -> Result<T> {
    id.to_string()
        .parse()
        .map_err(|e: DomainError| PortError::Tool(e.to_string()))
}
pub fn wire_error(error: PortError) -> ErrorEnvelope {
    let code = match &error {
        PortError::Storage(_) => ErrorCode::Storage,
        PortError::Policy(_) => ErrorCode::PolicyDenied,
        PortError::Tool(_) => ErrorCode::Tool,
        PortError::Cancelled => ErrorCode::Cancelled,
        PortError::TimedOut => ErrorCode::TimedOut,
        PortError::Busy => ErrorCode::Busy,
        PortError::NotFound => ErrorCode::NotFound,
        PortError::Conflict => ErrorCode::Conflict,
    };
    ErrorEnvelope::new(code, error.to_string())
}
/// Validates the schema subset emitted by this registry, including nested batch edits.
fn validate_input(input: &serde_json::Value, schema: &serde_json::Value) -> Result<()> {
    let type_valid = |kind: &str| match kind {
        "object" => input.is_object(),
        "array" => input.is_array(),
        "string" => input.is_string(),
        "integer" => input.is_i64() || input.is_u64(),
        "boolean" => input.is_boolean(),
        "null" => input.is_null(),
        _ => false,
    };
    if let Some(kind) = schema["type"].as_str()
        && !type_valid(kind)
    {
        return Err(PortError::Tool("invalid input type".into()));
    }
    if let Some(kinds) = schema["type"].as_array()
        && !kinds.iter().filter_map(|k| k.as_str()).any(type_valid)
    {
        return Err(PortError::Tool("invalid input type".into()));
    }
    if let Some(values) = schema["enum"].as_array()
        && !values.contains(input)
    {
        return Err(PortError::Tool("value outside enum".into()));
    }
    if let Some(number) = input.as_i64()
        && (schema["minimum"].as_i64().is_some_and(|min| number < min)
            || schema["maximum"].as_i64().is_some_and(|max| number > max))
    {
        return Err(PortError::Tool("numeric limit".into()));
    }
    if let Some(value) = input.as_str()
        && schema["format"] == "uuid"
        && uuid::Uuid::parse_str(value).is_err()
    {
        return Err(PortError::Tool("invalid UUID".into()));
    }
    if let Some(object) = input.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(|k| k.as_str()) {
                if !object.contains_key(key) {
                    return Err(PortError::Tool(format!("required field: {key}")));
                }
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (key, value) in object {
                match properties.get(key) {
                    Some(child) => validate_input(value, child)?,
                    None if schema["additionalProperties"] == false => {
                        return Err(PortError::Tool(format!("unknown field: {key}")));
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(array) = input.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|n| array.len() < n as usize)
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|n| array.len() > n as usize)
        {
            return Err(PortError::Tool("array limit".into()));
        }
        if let Some(child) = schema.get("items") {
            for value in array {
                validate_input(value, child)?;
            }
        }
    }
    Ok(())
}
