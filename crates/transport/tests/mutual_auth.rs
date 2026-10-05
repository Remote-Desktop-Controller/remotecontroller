use runtime_protocol::*;
use runtime_transport::*;

#[tokio::test]
async fn rogue_listener_never_receives_secret_or_trusted_request() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\rdc-rogue-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = directory
        .path()
        .join("rogue.sock")
        .to_string_lossy()
        .into_owned();
    let mut listener = Listener::bind(&endpoint).await.unwrap();
    let workspace = uuid::Uuid::new_v4();
    let captured = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let first = receive(&mut stream).await.unwrap();
        let bytes = rmp_serde::to_vec_named(&first).unwrap();
        send(
            &mut stream,
            &Frame::Welcome {
                protocol_version: VERSION,
                workspace_id: workspace,
            },
        )
        .await
        .unwrap();
        let next = receive(&mut stream).await.ok();
        let trusted_request = if let Some(Frame::Request(request)) = next {
            send(
                &mut stream,
                &Frame::Response(ResponseEnvelope {
                    meta: request.meta,
                    payload: Ok(serde_json::json!({"spoofed":true})),
                }),
            )
            .await
            .unwrap();
            true
        } else {
            false
        };
        (bytes, trusted_request)
    });
    let request = RequestEnvelope {
        meta: Metadata::new(Some(workspace), 2000),
        payload: Request::Catalog,
    };
    let result = Client::new(endpoint, vec![0x5c; 32])
        .request(&request, |_| {})
        .await;
    let (bytes, trusted_request) = captured.await.unwrap();
    assert!(
        result.is_err(),
        "Client accepted an unauthenticated IPC server"
    );
    assert!(
        !bytes.windows(32).any(|chunk| chunk == [0x5c; 32]),
        "Secret was sent in the initial handshake"
    );
    assert!(
        !trusted_request,
        "Authenticated tool request was sent to rogue server"
    );
    drop(directory);
}
