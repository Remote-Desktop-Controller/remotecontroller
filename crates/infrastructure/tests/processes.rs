use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel, processes::ProgramRule};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
#[test]
fn executable_fingerprint_has_a_measured_runtime_cost() {
    let start = std::time::Instant::now();
    let hash = ProgramRule::fingerprint(&std::env::current_exe().unwrap()).unwrap();
    assert_eq!(hash.len(), 64);
    eprintln!("fingerprint_ms={}", start.elapsed().as_millis());
}

// Launched only as a separate child by the integration tests below.
#[test]
#[ignore = "process fixture, intentionally sleeps until killed"]
fn sleeper() {
    println!("child-ready");
    std::thread::sleep(std::time::Duration::from_secs(20));
}
#[test]
#[ignore = "process tree fixture"]
fn tree_parent() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "sleeper", "--ignored", "--nocapture"])
        .spawn()
        .unwrap();
    println!("descendant-pid={}", child.id());
    let _ = child.wait();
}

fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let result = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        result != 0 && code == 259
    }
    #[cfg(unix)]
    {
        #[cfg(target_os = "linux")]
        if std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|s| {
                s.split_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with("Z "))
            })
        {
            return false;
        }
        // SAFETY: signal zero only checks the fixture child's existence.
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}
struct Progress {
    tx: mpsc::Sender<String>,
}
impl ProgressPort for Progress {
    fn emit(&self, _: usize, _: usize, message: &str) {
        let _ = self.tx.try_send(message.into());
    }
}
async fn fixture(
    timeout: u64,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<Kernel>,
    String,
    Vec<String>,
) {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let args = vec![
        "--exact".into(),
        "sleeper".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    let config = RuntimeConfig {
        #[cfg(unix)]
        guardian_path: Some(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("local-daemon"),
        ),
        operation_timeout_ms: timeout,
        allowed_programs: vec![ProgramRule {
            program: executable.clone(),
            args: args.clone(),
            fingerprint: Some(ProgramRule::fingerprint(&executable).unwrap()),
            trusted_unconfined: true,
        }],
        ..RuntimeConfig::default()
    };
    let kernel = Kernel::open(
        root.path(),
        state.path(),
        config,
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::SpawnProcess,
            Capability::KillProcess,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    (
        root,
        state,
        kernel,
        executable
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string(),
        args,
    )
}
#[tokio::test]
async fn operation_cancellation_kills_child_and_records_cancelled() {
    let (_root, _state, k, program, args) = fixture(10_000).await;
    let request = RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
        payload: Request::Execute {
            tool: "process.spawn".into(),
            input: json!({"program":program,"args":args}),
        },
    };
    let operation = request.meta.operation_id.to_string().parse().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let worker_k = k.clone();
    let worker =
        tokio::spawn(async move { worker_k.handle(request, Arc::new(Progress { tx })).await });
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let process_id = message
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("process_id=")
        .parse()
        .unwrap();
    assert_eq!(
        k.services
            .processes
            .status(k.workspace.id, process_id, None)
            .await
            .unwrap()["status"],
        "Running"
    );
    k.executor.cancel(k.workspace.id, operation).await.unwrap();
    assert_eq!(
        worker.await.unwrap().payload.unwrap()["status"],
        "Cancelled"
    );
    let recovered = k
        .services
        .processes
        .status(k.workspace.id, process_id, Some(false))
        .await
        .unwrap();
    assert!(
        recovered["stdout_ref"].is_string(),
        "durable stdout reference must survive record eviction"
    );
    assert_eq!(
        k.services
            .processes
            .status(k.workspace.id, process_id, None)
            .await
            .unwrap()["status"],
        "Cancelled"
    );
    assert!(
        k.services
            .processes
            .kill(k.workspace.id, process_id)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn deadline_times_out_child_and_unlisted_arguments_are_denied() {
    let (_root, _state, k, program, args) = fixture(250).await;
    let result = k
        .handle(
            RequestEnvelope {
                meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
                payload: Request::Execute {
                    tool: "process.spawn".into(),
                    input: json!({"program":program,"args":args}),
                },
            },
            Arc::new(NoProgress),
        )
        .await
        .payload
        .unwrap();
    assert_eq!(result["status"], "TimedOut");
    let denied = k
        .handle(
            RequestEnvelope {
                meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
                payload: Request::Execute {
                    tool: "process.spawn".into(),
                    input: json!({"program":program,"args":["unlisted"]}),
                },
            },
            Arc::new(NoProgress),
        )
        .await
        .payload
        .unwrap();
    assert_eq!(denied["status"], "Failed");
}
#[tokio::test]
async fn scheduler_applies_backpressure_to_live_operations() {
    let (root, state, k, program, args) = fixture(10_000).await;
    let mut config = k.config.clone();
    k.executor.shutdown().await;
    drop(k);
    config.scheduler.queue = 1;
    config.scheduler.processes = 1;
    let k = Kernel::open(
        root.path(),
        state.path(),
        config,
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::SpawnProcess,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    let request = RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
        payload: Request::Execute {
            tool: "process.spawn".into(),
            input: json!({"program":program,"args":args}),
        },
    };
    let operation = request.meta.operation_id.to_string().parse().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let worker_k = k.clone();
    let request2 = request.clone();
    let worker =
        tokio::spawn(async move { worker_k.handle(request, Arc::new(Progress { tx })).await });
    rx.recv().await.unwrap();
    let mut request2 = request2;
    request2.meta = Metadata::new(Some(k.workspace.id.as_uuid()), 10_000);
    assert_eq!(
        k.handle(request2, Arc::new(NoProgress))
            .await
            .payload
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    k.executor.cancel(k.workspace.id, operation).await.unwrap();
    assert_eq!(
        worker.await.unwrap().payload.unwrap()["status"],
        "Cancelled"
    );
}

#[tokio::test]
async fn executable_changed_after_local_approval_is_denied() {
    let (root, state, k, _, _) = fixture(10_000).await;
    let program = root.path().join("approved-fixture.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
    let mut config = k.config.clone();
    config.allowed_programs = vec![ProgramRule {
        fingerprint: Some(ProgramRule::fingerprint(&program).unwrap()),
        program: program.clone(),
        args: vec![],
        trusted_unconfined: true,
    }];
    std::fs::write(&program, b"executable replaced after local approval").unwrap();
    let result = Kernel::open(root.path(), state.path(), config, ExecutionPolicy::new([])).await;
    assert!(matches!(result, Err(PortError::Policy(_))));
    k.executor.shutdown().await;
}

#[test]
fn legacy_rule_deserialization_does_not_silently_approve_unconfined_execution() {
    let rule: ProgramRule =
        serde_json::from_value(json!({"program":"/bin/approved", "args":[]})).unwrap();
    assert!(!rule.trusted_unconfined);
    assert!(rule.fingerprint.is_none());
}

#[tokio::test]
async fn cancellation_terminates_descendant_tree() {
    let (root, state, k, program, mut args) = fixture(10_000).await;
    let mut config = k.config.clone();
    args[1] = "tree_parent".into();
    config.allowed_programs[0].args = args.clone();
    k.executor.shutdown().await;
    drop(k);
    let k = Kernel::open(
        root.path(),
        state.path(),
        config,
        ExecutionPolicy::new([
            Capability::ReadWorkspace,
            Capability::SpawnProcess,
            Capability::KillProcess,
            Capability::OutsideWorkspaceAccess,
            Capability::NetworkAccess,
        ]),
    )
    .await
    .unwrap();
    let request = RequestEnvelope {
        meta: Metadata::new(Some(k.workspace.id.as_uuid()), 10_000),
        payload: Request::Execute {
            tool: "process.spawn".into(),
            input: json!({"program":program,"args":args}),
        },
    };
    let operation = request.meta.operation_id.to_string().parse().unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let worker_k = k.clone();
    let worker =
        tokio::spawn(async move { worker_k.handle(request, Arc::new(Progress { tx })).await });
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let id = message
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("process_id=")
        .parse()
        .unwrap();
    let descendant = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let output = k
                .services
                .processes
                .status(k.workspace.id, id, Some(false))
                .await
                .unwrap();
            if let Some(pid) = output["output"].as_str().and_then(|s| {
                s.lines().find_map(|line| {
                    line.strip_prefix("descendant-pid=")
                        .and_then(|pid| pid.parse::<u32>().ok())
                })
            }) {
                break pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(process_alive(descendant));
    k.executor.cancel(k.workspace.id, operation).await.unwrap();
    assert_eq!(
        worker.await.unwrap().payload.unwrap()["status"],
        "Cancelled"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while process_alive(descendant) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("descendant survived cancellation");
    k.executor.shutdown().await;
}
