use crate::kernel::{ChannelProgress, Kernel};
use runtime_protocol::*;
use runtime_transport::{Listener, LocalStream, authenticate, receive, send};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
pub async fn serve(
    mut listener: Listener,
    kernel: Arc<Kernel>,
    secret: Vec<u8>,
    stop: CancellationToken,
) -> runtime_transport::Result<()> {
    let slots = Arc::new(Semaphore::new(kernel.config.ipc_connections));
    let mut clients = JoinSet::new();
    loop {
        let permit = tokio::select! {_ = stop.cancelled()=>break,p=slots.clone().acquire_owned()=>p.map_err(|_|runtime_transport::IpcError::Unexpected)?};
        let stream = tokio::select! {_ = stop.cancelled()=>break,r=listener.accept()=>r?};
        let kernel = kernel.clone();
        let secret = secret.clone();
        clients.spawn(async move {
            let _permit = permit;
            if let Err(e) = connection(stream, kernel, secret).await {
                tracing::debug!(error=%e,"IPC connection ended");
            }
        });
        while clients.try_join_next().is_some() {}
    }
    kernel.executor.shutdown().await;
    clients.abort_all();
    while clients.join_next().await.is_some() {}
    Ok(())
}
async fn connection(
    mut stream: LocalStream,
    kernel: Arc<Kernel>,
    secret: Vec<u8>,
) -> runtime_transport::Result<()> {
    authenticate(&mut stream, &secret, kernel.workspace.id.as_uuid()).await?;
    let frame = tokio::time::timeout(Duration::from_secs(5), receive(&mut stream))
        .await
        .map_err(|_| runtime_transport::IpcError::Deadline)??;
    let Frame::Request(request) = frame else {
        return Err(runtime_transport::IpcError::Unexpected);
    };
    if request.meta.protocol_version != VERSION {
        return send(
            &mut stream,
            &Frame::Response(ResponseEnvelope {
                meta: request.meta,
                payload: Err(ErrorEnvelope::new(
                    ErrorCode::VersionMismatch,
                    "unsupported IPC version",
                )),
            }),
        )
        .await;
    }
    let (tx, mut rx) = mpsc::channel(kernel.config.progress_capacity);
    let progress = Arc::new(ChannelProgress {
        sender: tx,
        meta: request.meta.clone(),
    });
    let response = kernel.handle(request, progress);
    tokio::pin!(response);
    loop {
        tokio::select! {biased;
            r=&mut response=>{tokio::time::timeout(Duration::from_secs(5),send(&mut stream,&Frame::Response(r))).await.map_err(|_|runtime_transport::IpcError::Deadline)??;return Ok(());},
            Some(e)=rx.recv()=>{tokio::time::timeout(Duration::from_secs(1),send(&mut stream,&Frame::Event(e))).await.map_err(|_|runtime_transport::IpcError::Deadline)??;}
        }
    }
}
