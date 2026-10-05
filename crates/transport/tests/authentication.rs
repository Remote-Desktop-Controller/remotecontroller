use runtime_protocol::*;
use runtime_transport::*;
#[tokio::test]
async fn wrong_secret_and_wrong_version_are_rejected() {
    for (version, secret, expected) in [
        (1, vec![0; 32], ErrorCode::VersionMismatch),
        (99, vec![7; 32], ErrorCode::VersionMismatch),
    ] {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let task =
            tokio::spawn(async move { authenticate(&mut b, &[7; 32], uuid::Uuid::new_v4()).await });
        send(
            &mut a,
            &Frame::Hello {
                protocol_version: version,
                secret,
            },
        )
        .await
        .unwrap();
        match receive(&mut a).await.unwrap() {
            Frame::Error(e) => assert_eq!(e.code, expected),
            _ => panic!("handshake unexpectedly accepted"),
        }
        assert!(task.await.unwrap().is_err());
    }
}

#[tokio::test]
async fn wrong_client_proof_is_rejected_after_server_challenge() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let task =
        tokio::spawn(
            async move { authenticate(&mut server, &[7; 32], uuid::Uuid::new_v4()).await },
        );
    send(
        &mut client,
        &Frame::ClientHello {
            protocol_version: VERSION,
            client_nonce: [5; 32],
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        receive(&mut client).await.unwrap(),
        Frame::ServerChallenge { .. }
    ));
    send(
        &mut client,
        &Frame::ClientProof {
            client_proof: Proof([0; 32]),
        },
    )
    .await
    .unwrap();
    match receive(&mut client).await.unwrap() {
        Frame::Error(error) => assert_eq!(error.code, ErrorCode::Unauthorized),
        _ => panic!("invalid proof accepted"),
    }
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn handshake_length_is_rejected_before_reading_or_allocating_body() {
    use tokio::io::AsyncWriteExt;
    let (mut client, mut server) = tokio::io::duplex(64);
    let task =
        tokio::spawn(
            async move { authenticate(&mut server, &[7; 32], uuid::Uuid::new_v4()).await },
        );
    client
        .write_u32((HANDSHAKE_MAX_FRAME + 1) as u32)
        .await
        .unwrap();
    assert!(matches!(task.await.unwrap(), Err(IpcError::FrameLimit)));
}

#[tokio::test]
async fn server_proof_cannot_be_reflected_as_client_proof() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let task =
        tokio::spawn(
            async move { authenticate(&mut server, &[7; 32], uuid::Uuid::new_v4()).await },
        );
    send(
        &mut client,
        &Frame::ClientHello {
            protocol_version: VERSION,
            client_nonce: [6; 32],
        },
    )
    .await
    .unwrap();
    let Frame::ServerChallenge { server_proof, .. } = receive(&mut client).await.unwrap() else {
        panic!("missing challenge")
    };
    send(
        &mut client,
        &Frame::ClientProof {
            client_proof: server_proof,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        receive(&mut client).await.unwrap(),
        Frame::Error(ErrorEnvelope {
            code: ErrorCode::Unauthorized,
            ..
        })
    ));
    assert!(task.await.unwrap().is_err());
}
