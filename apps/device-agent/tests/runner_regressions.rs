//! Availability regressions through real HTTP and mutually authenticated local IPC.
use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use device_agent::{
    http::{RemoteCommand, RemoteResult},
    profile::{self, Profile},
    runner,
};
use runtime_protocol::{Frame, Metadata, Request, RequestEnvelope, ResponseEnvelope};
use runtime_transport::{Client, Listener, authenticate, receive, send};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Observed {
    normal_active: AtomicUsize,
    normal_started: AtomicUsize,
    polls: AtomicUsize,
    heartbeats: AtomicUsize,
    phase: AtomicUsize,
    commands: Mutex<Vec<RemoteCommand>>,
    control_results: Mutex<Vec<RemoteResult>>,
    statuses: Mutex<HashMap<uuid::Uuid, u16>>,
    normal_release: CancellationToken,
    status_seen: CancellationToken,
}

async fn heartbeat(State(state): State<Arc<Observed>>, Json(_): Json<Value>) -> Json<Value> {
    state.heartbeats.fetch_add(1, Ordering::SeqCst);
    Json(json!({"status":"ok"}))
}
async fn poll(State(state): State<Arc<Observed>>) -> Json<Value> {
    state.polls.fetch_add(1, Ordering::SeqCst);
    let phase = state.phase.load(Ordering::SeqCst);
    let commands = if phase == 0
        && state
            .phase
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        state.commands.lock().unwrap().iter().filter(|command|matches!(&command.envelope.payload,Request::Execute{tool,..} if tool=="process.wait")).cloned().collect::<Vec<_>>()
    } else if phase == 1
        && state.normal_started.load(Ordering::SeqCst) == 4
        && state
            .phase
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        state.commands.lock().unwrap().iter().filter(|command|matches!(&command.envelope.payload,Request::Execute{tool,..} if tool=="operation.cancel"||tool=="operation.status")).cloned().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    // Avoid a spinning HTTP fixture while retaining a bounded test deadline.
    if commands.is_empty() {
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    Json(
        json!({"commands":commands.iter().map(|c|json!({"request_id":c.request_id,"envelope":c.envelope})).collect::<Vec<_>>()}),
    )
}
async fn result(
    State(state): State<Arc<Observed>>,
    Json(result): Json<RemoteResult>,
) -> (StatusCode, Json<Value>) {
    let status = state
        .statuses
        .lock()
        .unwrap()
        .get(&result.request_id)
        .copied()
        .unwrap_or(200);
    if status == 200 {
        state.control_results.lock().unwrap().push(result);
    }
    (
        StatusCode::from_u16(status).unwrap(),
        Json(if status == 200 {
            json!({"ok":true})
        } else {
            json!({"error":"fixture_status","message":"Result status fixture"})
        }),
    )
}
struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    profile: Profile,
    ipc: Client,
    observed: Arc<Observed>,
    http_task: tokio::task::JoinHandle<()>,
    ipc_task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.http_task.abort();
        self.ipc_task.abort();
        self.observed.normal_release.cancel();
    }
}
async fn fixture() -> Result<Fixture> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("private");
    runtime_transport::private_directory(&root)?;
    let workspace_path = directory.path().join("workspace");
    std::fs::create_dir(&workspace_path)?;
    let workspace = uuid::Uuid::new_v4();
    let endpoint = if cfg!(windows) {
        format!(r"\\.\pipe\rdc-agent-regression-{}", uuid::Uuid::new_v4())
    } else {
        directory
            .path()
            .join("ipc.sock")
            .to_string_lossy()
            .into_owned()
    };
    let mut listener = Listener::bind(&endpoint).await?;
    let observed = Arc::new(Observed::default());
    let daemon_state = observed.clone();
    let ipc_task = tokio::spawn(async move {
        while let Ok(mut stream) = listener.accept().await {
            let state = daemon_state.clone();
            tokio::spawn(async move {
                authenticate(&mut stream, &[17; 32], workspace)
                    .await
                    .unwrap();
                let request = match receive(&mut stream).await.unwrap() {
                    Frame::Request(request) => request,
                    _ => panic!("Expected authenticated request"),
                };
                let payload = match &request.payload {
                    Request::Catalog => json!({"workspace_id":workspace,"tools":[]}),
                    Request::Execute { tool, .. } if tool == "process.wait" => {
                        state.normal_active.fetch_add(1, Ordering::SeqCst);
                        state.normal_started.fetch_add(1, Ordering::SeqCst);
                        state.normal_release.cancelled().await;
                        state.normal_active.fetch_sub(1, Ordering::SeqCst);
                        json!({"cancelled":true})
                    }
                    Request::Execute { tool, .. } if tool == "operation.status" => {
                        let active = state.normal_active.load(Ordering::SeqCst);
                        state.status_seen.cancel();
                        json!({"normal_active":active,"control":"status"})
                    }
                    Request::Execute { tool, .. } if tool == "operation.cancel" => {
                        // Status and cancellation must both traverse the reserved lane.
                        state.status_seen.cancelled().await;
                        let active = state.normal_active.load(Ordering::SeqCst);
                        state.normal_release.cancel();
                        json!({"normal_active":active,"control":"cancel"})
                    }
                    _ => json!({"ok":true}),
                };
                let _ = send(
                    &mut stream,
                    &Frame::Response(ResponseEnvelope {
                        meta: request.meta,
                        payload: Ok(payload),
                    }),
                )
                .await;
            });
        }
    });
    let web = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", web.local_addr()?);
    let router = Router::new()
        .route("/api/agent/heartbeat", post(heartbeat))
        .route("/api/agent/poll", get(poll))
        .route("/api/agent/result", post(result))
        .with_state(observed.clone());
    let http_task = tokio::spawn(async move {
        axum::serve(web, router).await.unwrap();
    });
    let profile = Profile {
        server: origin,
        device_id: uuid::Uuid::new_v4(),
        device_token: "private-device-token-01234567890123456789".into(),
        workspace: workspace_path,
        state_dir: root.join("runtime"),
        endpoint: endpoint.clone(),
        allow_http_loopback: true,
        allow_write: false,
        allow_process: false,
        allow_terminal: false,
        allow_process_network: false,
        allow_process_outside_workspace: false,
    };
    Ok(Fixture {
        _directory: directory,
        root,
        profile,
        ipc: Client::new(endpoint, vec![17; 32]),
        observed,
        http_task,
        ipc_task,
    })
}
async fn wait_until(mut condition: impl FnMut() -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    Ok(())
}
fn stored_result(request_id: uuid::Uuid) -> RemoteResult {
    RemoteResult {
        request_id,
        envelope: ResponseEnvelope {
            meta: Metadata::new(None, 60_000),
            payload: Ok(json!({"private_output":"Auditoria local preservada"})),
        },
    }
}
fn preload(fixture: &Fixture, result: &RemoteResult, status: u16) -> Result<()> {
    let outbox = fixture.root.join("outbox");
    runtime_transport::private_directory(&outbox)?;
    profile::write_private(
        &outbox.join(format!("{}.json", result.request_id)),
        &serde_json::to_vec(result)?,
    )?;
    fixture
        .observed
        .statuses
        .lock()
        .unwrap()
        .insert(result.request_id, status);
    Ok(())
}

#[tokio::test]
async fn execute_control_tools_reach_ipc_while_four_normal_workers_are_blocked() -> Result<()> {
    let fixture = fixture().await?;
    let (workspace, _) = runner::catalog(&fixture.ipc).await?;
    let mut commands = Vec::new();
    for tool in [
        "process.wait",
        "process.wait",
        "process.wait",
        "process.wait",
        "process.wait",
        "operation.status",
        "operation.cancel",
    ] {
        let meta = Metadata::new(Some(workspace), 60_000);
        let request_id = meta.request_id;
        commands.push(RemoteCommand {
            request_id,
            envelope: RequestEnvelope {
                meta,
                payload: Request::Execute {
                    tool: tool.into(),
                    input: json!({"operation_id":uuid::Uuid::new_v4()}),
                },
            },
        });
    }
    *fixture.observed.commands.lock().unwrap() = commands;
    let stop = CancellationToken::new();
    let task = tokio::spawn(runner::run(
        fixture.profile.clone(),
        fixture.ipc.clone(),
        fixture.root.clone(),
        stop.clone(),
    ));
    wait_until(|| {
        fixture
            .observed
            .control_results
            .lock()
            .unwrap()
            .iter()
            .filter(|result| {
                result
                    .envelope
                    .payload
                    .as_ref()
                    .is_ok_and(|value| value["control"].is_string())
            })
            .count()
            == 2
    })
    .await?;
    for result in fixture.observed.control_results.lock().unwrap().iter() {
        if let Ok(value) = &result.envelope.payload
            && value["control"].is_string()
        {
            assert_eq!(
                value["normal_active"], 4,
                "control was delayed until normal capacity became free"
            );
        }
    }
    assert_eq!(fixture.observed.normal_started.load(Ordering::SeqCst), 4);
    stop.cancel();
    task.await??;
    Ok(())
}

#[tokio::test]
async fn expired_results_are_privately_archived_and_do_not_block_heartbeat_or_poll() -> Result<()> {
    let fixture = fixture().await?;
    let expired = stored_result(uuid::Uuid::new_v4());
    let gone = stored_result(uuid::Uuid::new_v4());
    let temporary = stored_result(uuid::Uuid::new_v4());
    let delivered = stored_result(uuid::Uuid::new_v4());
    for (result, status) in [
        (&expired, 404),
        (&gone, 410),
        (&temporary, 503),
        (&delivered, 200),
    ] {
        preload(&fixture, result, status)?;
    }
    let stop = CancellationToken::new();
    let task = tokio::spawn(runner::run(
        fixture.profile.clone(),
        fixture.ipc.clone(),
        fixture.root.clone(),
        stop.clone(),
    ));
    wait_until(|| {
        fixture.observed.polls.load(Ordering::SeqCst) > 1
            && fixture
                .root
                .join("outbox-archive")
                .join(format!("{}.json", gone.request_id))
                .exists()
    })
    .await?;
    assert!(fixture.observed.heartbeats.load(Ordering::SeqCst) > 0);
    for result in [&expired, &gone] {
        let path = fixture
            .root
            .join("outbox-archive")
            .join(format!("{}.json", result.request_id));
        let record: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        assert_eq!(record["reason"], "remote_request_expired");
        assert_eq!(record["result"], serde_json::to_value(result)?);
        assert!(
            !fixture
                .root
                .join("outbox")
                .join(format!("{}.json", result.request_id))
                .exists()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        }
    }
    assert_eq!(
        std::fs::read(
            fixture
                .root
                .join("outbox")
                .join(format!("{}.json", temporary.request_id))
        )?,
        serde_json::to_vec(&temporary)?
    );
    assert!(
        !fixture
            .root
            .join("outbox")
            .join(format!("{}.json", delivered.request_id))
            .exists()
    );
    stop.cancel();
    task.await??;
    Ok(())
}

#[tokio::test]
async fn revoked_credentials_stop_the_agent_without_discarding_undelivered_audit() -> Result<()> {
    for status in [401, 403] {
        let fixture = fixture().await?;
        let result = stored_result(uuid::Uuid::new_v4());
        preload(&fixture, &result, status)?;
        let stopped = tokio::time::timeout(
            Duration::from_secs(5),
            runner::run(
                fixture.profile.clone(),
                fixture.ipc.clone(),
                fixture.root.clone(),
                CancellationToken::new(),
            ),
        )
        .await?;
        assert!(stopped.unwrap_err().to_string().contains("AGENT_REVOKED"));
        assert_eq!(
            std::fs::read(
                fixture
                    .root
                    .join("outbox")
                    .join(format!("{}.json", result.request_id))
            )?,
            serde_json::to_vec(&result)?
        );
    }
    Ok(())
}

#[tokio::test]
async fn full_archive_preserves_new_expired_result_and_still_polls() -> Result<()> {
    let fixture = fixture().await?;
    let archive = fixture.root.join("outbox-archive");
    runtime_transport::private_directory(&archive)?;
    for _ in 0..64 {
        let result = stored_result(uuid::Uuid::new_v4());
        profile::write_private(
            &archive.join(format!("{}.json", result.request_id)),
            &serde_json::to_vec(
                &json!({"version":1,"reason":"remote_request_expired","http_status":404,"archived_at":1,"result":result}),
            )?,
        )?;
    }
    let result = stored_result(uuid::Uuid::new_v4());
    preload(&fixture, &result, 410)?;
    let stop = CancellationToken::new();
    let task = tokio::spawn(runner::run(
        fixture.profile.clone(),
        fixture.ipc.clone(),
        fixture.root.clone(),
        stop.clone(),
    ));
    wait_until(|| fixture.observed.polls.load(Ordering::SeqCst) > 1).await?;
    assert_eq!(std::fs::read_dir(&archive)?.count(), 64);
    assert_eq!(
        std::fs::read(
            fixture
                .root
                .join("outbox")
                .join(format!("{}.json", result.request_id))
        )?,
        serde_json::to_vec(&result)?
    );
    stop.cancel();
    task.await??;
    Ok(())
}

#[tokio::test]
async fn temporary_result_backlog_does_not_starve_a_later_healthy_delivery() -> Result<()> {
    let fixture = fixture().await?;
    for index in 1..=6 {
        preload(&fixture, &stored_result(uuid::Uuid::from_u128(index)), 503)?;
    }
    let healthy = stored_result(uuid::Uuid::from_u128(7));
    preload(&fixture, &healthy, 200)?;
    let stop = CancellationToken::new();
    let task = tokio::spawn(runner::run(
        fixture.profile.clone(),
        fixture.ipc.clone(),
        fixture.root.clone(),
        stop.clone(),
    ));
    wait_until(|| {
        fixture
            .observed
            .control_results
            .lock()
            .unwrap()
            .iter()
            .any(|result| result.request_id == healthy.request_id)
    })
    .await?;
    assert!(fixture.observed.heartbeats.load(Ordering::SeqCst) > 0);
    assert_eq!(
        std::fs::read_dir(fixture.root.join("outbox"))?.count(),
        6,
        "transient failures must remain durable"
    );
    stop.cancel();
    task.await??;
    Ok(())
}
