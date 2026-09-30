use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel, server::serve};
use runtime_protocol::*;
use runtime_transport::{Client, Listener};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn local_ipc_round_trip_progress_version_and_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\rdc-test-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = state.path().join("test.sock").to_string_lossy().to_string();
    let policy = ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]);
    let kernel = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        policy.clone(),
    )
    .await
    .unwrap();
    let workspace = kernel.workspace.id.as_uuid();
    let stop = CancellationToken::new();
    let listener = Listener::bind(&endpoint).await.unwrap();
    let server = tokio::spawn(serve(listener, kernel.clone(), vec![42; 32], stop.clone()));
    let client = Client::new(endpoint.clone(), vec![42; 32]);
    let catalog = client
        .request(
            &RequestEnvelope {
                meta: Metadata::new(None, 5000),
                payload: Request::Catalog,
            },
            |_| {},
        )
        .await
        .unwrap()
        .payload
        .unwrap();
    assert!(catalog["tools"].as_array().unwrap().len() >= 23);
    let req = RequestEnvelope {
        meta: Metadata::new(Some(workspace), 10_000),
        payload: Request::Execute {
            tool: "filesystem.write".into(),
            input: json!({"path":"ipc","content":"durable"}),
        },
    };
    let mut progress = 0;
    assert_eq!(
        client
            .request(&req, |_| progress += 1)
            .await
            .unwrap()
            .payload
            .unwrap()["status"],
        "Succeeded"
    );
    assert!(progress > 0);
    let mut mismatch = req.clone();
    mismatch.meta.protocol_version = 99;
    assert_eq!(
        client
            .request(&mismatch, |_| {})
            .await
            .unwrap()
            .payload
            .unwrap_err()
            .code,
        ErrorCode::VersionMismatch
    );
    stop.cancel();
    server.await.unwrap().unwrap();
    drop(kernel);
    let restarted = Kernel::open(root.path(), state.path(), RuntimeConfig::default(), policy)
        .await
        .unwrap();
    let stop = CancellationToken::new();
    let listener = Listener::bind(&endpoint).await.unwrap();
    let server = tokio::spawn(serve(listener, restarted, vec![42; 32], stop.clone()));
    assert_eq!(
        client.request(&req, |_| {}).await.unwrap().payload.unwrap()["status"],
        "Succeeded"
    );
    let unauthorized = Client::new(endpoint, vec![0; 32]);
    assert!(
        unauthorized
            .request(
                &RequestEnvelope {
                    meta: Metadata::new(None, 5000),
                    payload: Request::Heartbeat
                },
                |_| {}
            )
            .await
            .is_err()
    );
    stop.cancel();
    server.await.unwrap().unwrap();
}
