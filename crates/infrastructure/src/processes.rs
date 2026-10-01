use crate::{
    files::WorkspaceFiles,
    storage::{LocalStore, storage_error},
};
use async_trait::async_trait;
use runtime_domain::*;
use runtime_ports::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;
mod process_spool;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramRule {
    pub program: PathBuf,
    pub args: Vec<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub trusted_unconfined: bool,
}
impl ProgramRule {
    /// Intended for explicit local administrative approval, never exposed as an MCP tool.
    pub fn fingerprint(program: &std::path::Path) -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(program).map_err(crate::files::io_error)?;
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 65536];
        loop {
            let n = file.read(&mut bytes).map_err(crate::files::io_error)?;
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}
struct ProcessState {
    workspace: WorkspaceId,
    operation: OperationId,
    pid: u32,
    status: String,
    exit: Option<i32>,
    stdout: VecDeque<u8>,
    stderr: VecDeque<u8>,
    stdout_bytes: u64,
    stderr_bytes: u64,
    token: CancellationToken,
}
pub struct ProcessManager {
    files: WorkspaceFiles,
    store: Arc<LocalStore>,
    rules: Vec<ProgramRule>,
    states: RwLock<HashMap<ProcessId, Arc<Mutex<ProcessState>>>>,
    buffer_bytes: usize,
    max_records: usize,
    spool: process_spool::SpoolBudget,
    guardian: Option<PathBuf>,
}
impl ProcessManager {
    pub fn new(
        files: WorkspaceFiles,
        store: Arc<LocalStore>,
        rules: Vec<ProgramRule>,
        buffer_bytes: usize,
    ) -> Result<Self> {
        let rules = rules
            .into_iter()
            .map(|mut r| {
                r.program = r.program.canonicalize().map_err(crate::files::io_error)?;
                if r.trusted_unconfined && r.fingerprint.as_deref() != Some(&ProgramRule::fingerprint(&r.program)?) {
                    return Err(PortError::Policy("approved executable fingerprint missing or changed; local reapproval required".into()));
                }
                let name = r
                    .program
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if matches!(
                    name.as_str(),
                    "cmd" | "powershell" | "pwsh" | "sh" | "bash" | "zsh"
                ) {
                    return Err(PortError::Policy(
                        "privileged shells are not supported".into(),
                    ));
                }
                Ok(r)
            })
            .collect::<Result<_>>()?;
        let spool = process_spool::SpoolBudget::new(
            &store.state_dir().join("process-spool"),
            16 * 1024 * 1024,
            256 * 1024 * 1024,
        )
        .map_err(crate::files::io_error)?;
        Ok(Self {
            files,
            store,
            rules,
            states: RwLock::new(HashMap::new()),
            buffer_bytes: buffer_bytes.clamp(1024, 1024 * 1024),
            max_records: 128,
            spool,
            guardian: None,
        })
    }
    pub fn with_guardian(mut self, executable: PathBuf) -> Result<Self> {
        self.guardian = Some(executable.canonicalize().map_err(crate::files::io_error)?);
        Ok(self)
    }
    pub fn with_spool_limits(mut self, per_process_bytes: u64, total_bytes: u64) -> Result<Self> {
        self.spool = process_spool::SpoolBudget::new(
            &self.store.state_dir().join("process-spool"),
            per_process_bytes,
            total_bytes,
        )
        .map_err(crate::files::io_error)?;
        Ok(self)
    }
    async fn enrich(
        &self,
        id: ProcessId,
        stream: Option<bool>,
        mut result: Value,
    ) -> Result<Value> {
        let spool = self.spool.clone();
        let capacity = self.buffer_bytes;
        let saved = tokio::task::spawn_blocking(move || {
            spool.read(&id.to_string(), stream.unwrap_or(false), capacity)
        })
        .await
        .map_err(storage_error)?;
        match saved {
            Ok(saved) => {
                if stream.is_some() {
                    result["output"] = json!(String::from_utf8_lossy(&saved.output));
                }
                result["stdout_bytes"] = json!(saved.metadata.stdout_bytes);
                result["stderr_bytes"] = json!(saved.metadata.stderr_bytes);
                result["stdout_stored_bytes"] = json!(saved.metadata.stdout_stored);
                result["stderr_stored_bytes"] = json!(saved.metadata.stderr_stored);
                result["truncated"] = json!(
                    saved.metadata.truncated
                        || saved.metadata.stdout_stored > capacity as u64
                        || saved.metadata.stderr_stored > capacity as u64
                );
                result["capture_incomplete"] = json!(
                    saved.metadata.capture_incomplete
                        || result["status"] == "interrupted"
                        || result["status"] == "CaptureFailed"
                );
                result["stdout_ref"] = json!(format!("process-spool/{id}/stdout"));
                result["stderr_ref"] = json!(format!("process-spool/{id}/stderr"));
                result["execution_profile"] = json!("trusted_unconfined");
                result["duration_ms"] = json!(saved.metadata.duration_ms);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                result["capture_incomplete"] = json!(true);
            }
            Err(error) => return Err(crate::files::io_error(error)),
        }
        Ok(result)
    }
    pub async fn reconcile(&self) -> Result<()> {
        self.store
            .run(|c| async move {
                // Never kill a persisted PID blindly: PIDs can be reused after restart.
                c.execute(
                    "UPDATE processes SET status='interrupted' WHERE status IN ('Starting','Running')",
                    (),
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await
    }
    async fn drain<R: tokio::io::AsyncRead + Unpin>(
        mut reader: R,
        state: Arc<Mutex<ProcessState>>,
        stderr: bool,
        capacity: usize,
        capture: Arc<std::sync::Mutex<process_spool::Capture>>,
    ) -> Result<()> {
        let mut bytes = [0u8; 8192];
        loop {
            let n = reader
                .read(&mut bytes)
                .await
                .map_err(crate::files::io_error)?;
            if n == 0 {
                break;
            }
            let persisted = bytes[..n].to_vec();
            let capture = capture.clone();
            tokio::task::spawn_blocking(move || {
                capture
                    .lock()
                    .map_err(|_| PortError::Storage("spool poisoned".into()))?
                    .append(stderr, &persisted)
                    .map_err(crate::files::io_error)
            })
            .await
            .map_err(storage_error)??;
            let mut state = state.lock().await;
            if stderr {
                state.stderr_bytes += n as u64;
            } else {
                state.stdout_bytes += n as u64;
            }
            let buffer = if stderr {
                &mut state.stderr
            } else {
                &mut state.stdout
            };
            buffer.extend(&bytes[..n]);
            while buffer.len() > capacity {
                buffer.pop_front();
            }
        }
        Ok(())
    }
}
#[async_trait]
impl ProcessPort for ProcessManager {
    async fn spawn(&self, spec: ProcessSpec, context: ToolContext) -> Result<Value> {
        if !std::path::Path::new(&spec.program).is_absolute() {
            return Err(PortError::Policy("absolute program path required".into()));
        }
        let requested_program = tokio::fs::canonicalize(&spec.program)
            .await
            .map_err(crate::files::io_error)?;
        let rule = self
            .rules
            .iter()
            .find(|r| r.program == requested_program && r.args == spec.args)
            .ok_or_else(|| {
                PortError::Policy(
                    "program and exact argument vector must be explicitly allowed".into(),
                )
            })?;
        if !rule.trusted_unconfined {
            return Err(PortError::Policy(
                "unconfined execution requires explicit local trusted approval".into(),
            ));
        }
        let fingerprint_program = rule.program.clone();
        let actual_fingerprint =
            tokio::task::spawn_blocking(move || ProgramRule::fingerprint(&fingerprint_program))
                .await
                .map_err(storage_error)??;
        if rule.fingerprint.as_deref() != Some(actual_fingerprint.as_str()) {
            return Err(PortError::Policy(
                "approved executable fingerprint changed; local reapproval required".into(),
            ));
        }
        #[cfg(unix)]
        if self.guardian.is_none() {
            return Err(PortError::Policy(
                "Unix execution requires configured lifecycle guardian".into(),
            ));
        }
        let cwd_relative = self.files.validate(&spec.cwd)?;
        let cwd = self
            .files
            .root()
            .join(cwd_relative)
            .canonicalize()
            .map_err(crate::files::io_error)?;
        if !cwd.starts_with(self.files.root()) {
            return Err(PortError::Policy("process cwd escapes workspace".into()));
        }
        {
            let states = self.states.read().await;
            if states.len() >= self.max_records {
                return Err(PortError::Busy);
            }
        }
        if context.cancellation.is_cancelled() {
            return Err(PortError::Cancelled);
        }
        let id = ProcessId::new();
        let now = runtime_protocol::now_ms() as i64;
        let started = std::time::Instant::now();
        let operation = context.operation_id;
        let workspace = context.workspace_id;
        let cwd_string = cwd.to_string_lossy().to_string();
        self.store.run(move|c|async move{
            c.execute("INSERT INTO processes(id,workspace_id,operation_id,cwd,started_at,status) VALUES(?1,?2,?3,?4,?5,'Starting')",libsql::params![id.to_string(),workspace.to_string(),operation.to_string(),cwd_string,now]).await.map_err(storage_error)?;Ok(())
        }).await?;
        let spool = self.spool.clone();
        let capture = tokio::task::spawn_blocking(move || {
            spool
                .create(&id.to_string())
                .map(|c| Arc::new(std::sync::Mutex::new(c)))
                .map_err(crate::files::io_error)
        })
        .await
        .map_err(storage_error)??;
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new(self.guardian.as_ref().expect("guardian checked"));
            command.arg("--process-guardian").arg(&rule.program);
            command
        };
        #[cfg(not(unix))]
        let mut command = Command::new(&rule.program);
        command
            .args(&spec.args)
            .current_dir(cwd)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.stdin(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            command.creation_flags(0x08000000 | 0x00000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
            if let Some(root) = std::env::var_os("SystemRoot") {
                command.env("SystemRoot", root);
            }
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(crate::files::io_error)?;
        #[cfg(unix)]
        let mut heartbeat = child.stdin.take();
        let pid = child
            .id()
            .ok_or_else(|| PortError::Tool("process lacks PID".into()))?;
        #[cfg(windows)]
        let containment = match platform::Job::attach(pid) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill().await;
                return Err(crate::files::io_error(e));
            }
        };
        #[cfg(windows)]
        if let Err(error) = platform::Job::resume_suspended(pid) {
            containment.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(crate::files::io_error(error));
        }
        let child_token = context.cancellation.child_token();
        let state = Arc::new(Mutex::new(ProcessState {
            workspace,
            operation,
            pid,
            status: "Running".into(),
            exit: None,
            stdout: VecDeque::new(),
            stderr: VecDeque::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            token: child_token.clone(),
        }));
        self.states.write().await.insert(id, state.clone());
        self.store
            .run(move |c| async move {
                c.execute(
                    "UPDATE processes SET os_pid=?2,status='Running' WHERE id=?1",
                    libsql::params![id.to_string(), pid as i64],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await?;
        context
            .progress
            .emit(0, 1, &format!("process_id={id}; pid={pid}"));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PortError::Tool("stdout unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PortError::Tool("stderr unavailable".into()))?;
        let mut out = tokio::spawn(Self::drain(
            stdout,
            state.clone(),
            false,
            self.buffer_bytes,
            capture.clone(),
        ));
        let mut err = tokio::spawn(Self::drain(
            stderr,
            state.clone(),
            true,
            self.buffer_bytes,
            capture.clone(),
        ));
        let mut cancelled = false;
        let status = tokio::select! {
            biased;
            _=child_token.cancelled()=>{
                cancelled=true;
                #[cfg(windows)]containment.terminate();
                #[cfg(unix)]drop(heartbeat.take());
                #[cfg(windows)]
                child.kill().await.map_err(crate::files::io_error)?;
                child.wait().await.map_err(crate::files::io_error)?
            }
            r=child.wait()=>r.map_err(crate::files::io_error)?,
        };
        #[cfg(windows)]
        drop(containment);
        #[cfg(unix)]
        drop(heartbeat.take());
        // Descendants must not keep stdout/stderr open forever.
        let capture_result = match tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let (outcome, errorcome) = tokio::join!(&mut out, &mut err);
            outcome.map_err(|e| PortError::Tool(e.to_string()))??;
            errorcome.map_err(|e| PortError::Tool(e.to_string()))??;
            Ok::<_, PortError>(())
        })
        .await
        {
            Ok(r) => r,
            Err(_) => {
                out.abort();
                err.abort();
                Err(PortError::Tool(
                    "process stream drain timed out; capture incomplete".into(),
                ))
            }
        };
        let duration = started.elapsed().as_millis() as u64;
        let incomplete = capture_result.is_err();
        let finish_result = tokio::task::spawn_blocking(move || {
            capture
                .lock()
                .map_err(|_| PortError::Storage("spool poisoned".into()))?
                .finish(duration, incomplete)
                .map_err(crate::files::io_error)
        })
        .await
        .map_err(storage_error)?;
        let mut s = state.lock().await;
        s.exit = status.code();
        s.status = if capture_result.is_err() || finish_result.is_err() {
            "CaptureFailed"
        } else if cancelled {
            "Cancelled"
        } else {
            "Exited"
        }
        .into();
        let exit = s.exit;
        let persisted_status = s.status.clone();
        let out_bytes: Vec<_> = s.stdout.iter().copied().collect();
        let err_bytes: Vec<_> = s.stderr.iter().copied().collect();
        self.store
            .run(move |c| async move {
                c.execute(
                    "UPDATE processes SET status=?2,exit_code=?3,stdout=?4,stderr=?5 WHERE id=?1",
                    libsql::params![id.to_string(), persisted_status, exit, out_bytes, err_bytes],
                )
                .await
                .map_err(storage_error)?;
                Ok(())
            })
            .await?;
        drop(s);
        let mut result = self.status(workspace, id, None).await?;
        result["duration_ms"] = json!(started.elapsed().as_millis() as u64);
        result["execution_profile"] = json!("trusted_unconfined");
        self.states.write().await.remove(&id);
        finish_result?;
        capture_result?;
        if cancelled {
            Err(PortError::Cancelled)
        } else if !status.success() {
            Err(PortError::Tool(format!(
                "program exited with {exit:?}; process_id={id}"
            )))
        } else {
            Ok(result)
        }
    }
    async fn status(
        &self,
        workspace: WorkspaceId,
        id: ProcessId,
        stream: Option<bool>,
    ) -> Result<Value> {
        let live = self.states.read().await.get(&id).cloned();
        if let Some(state) = live {
            let s = state.lock().await;
            if s.workspace != workspace {
                return Err(PortError::NotFound);
            }
            let output = match stream {
                Some(false) => Some(
                    String::from_utf8_lossy(&s.stdout.iter().copied().collect::<Vec<_>>())
                        .to_string(),
                ),
                Some(true) => Some(
                    String::from_utf8_lossy(&s.stderr.iter().copied().collect::<Vec<_>>())
                        .to_string(),
                ),
                None => None,
            };
            let result = json!({"process_id":id.to_string(),"operation_id":s.operation.to_string(),"pid":s.pid,"status":s.status,"exit_code":s.exit,"output":output,"stdout_bytes":s.stdout_bytes,"stderr_bytes":s.stderr_bytes,"truncated":s.stdout_bytes>self.buffer_bytes as u64||s.stderr_bytes>self.buffer_bytes as u64});
            drop(s);
            return self.enrich(id, stream, result).await;
        }
        let result = self.store.run(move|c|async move{
            let mut rows=c.query("SELECT operation_id,os_pid,status,exit_code,stdout,stderr FROM processes WHERE id=?1 AND workspace_id=?2",libsql::params![id.to_string(),workspace.to_string()]).await.map_err(storage_error)?;
            let row=rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?;
            let output=match stream{Some(err)=>{let bytes:Option<Vec<u8>>=row.get(if err{5}else{4}).map_err(storage_error)?;Some(String::from_utf8_lossy(&bytes.unwrap_or_default()).to_string())},None=>None};
            Ok(json!({"process_id":id.to_string(),"operation_id":row.get::<String>(0).map_err(storage_error)?,"pid":row.get::<Option<i64>>(1).map_err(storage_error)?,"status":row.get::<String>(2).map_err(storage_error)?,"exit_code":row.get::<Option<i64>>(3).map_err(storage_error)?,"output":output}))
        }).await?;
        self.enrich(id, stream, result).await
    }
    async fn kill(&self, workspace: WorkspaceId, id: ProcessId) -> Result<()> {
        let state = self
            .states
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(PortError::NotFound)?;
        let s = state.lock().await;
        if s.workspace != workspace {
            return Err(PortError::NotFound);
        }
        s.token.cancel();
        Ok(())
    }
}
#[cfg(windows)]
mod platform;
