use runtime_protocol::{Frame, MAX_FRAME};
use runtime_transport::{receive, send};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn framed_message_round_trip() {
    let (mut a, mut b) = tokio::io::duplex(4096);
    send(
        &mut a,
        &Frame::Hello {
            protocol_version: 1,
            secret: vec![17; 32],
        },
    )
    .await
    .unwrap();
    let frame = receive(&mut b).await.unwrap();
    assert!(matches!(
        frame,
        Frame::Hello {
            protocol_version: 1,
            ..
        }
    ));
}
#[tokio::test]
async fn excessive_frame_rejected_before_allocation() {
    let (mut a, mut b) = tokio::io::duplex(64);
    a.write_u32((MAX_FRAME + 1) as u32).await.unwrap();
    assert!(receive(&mut b).await.is_err());
}
