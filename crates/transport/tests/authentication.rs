use runtime_protocol::*;
use runtime_transport::*;
#[tokio::test]
async fn wrong_secret_and_wrong_version_are_rejected() {
    for (version, secret, expected) in [
        (VERSION, vec![0; 32], ErrorCode::Unauthorized),
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
