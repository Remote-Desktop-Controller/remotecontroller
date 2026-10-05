use crate::{
    http::{RemoteCommand, RemoteResult, ServiceClient, ServiceHttpError},
    profile::{self, Profile},
};
use anyhow::{Result, ensure};
use runtime_protocol::{
    ErrorCode, ErrorEnvelope, Metadata, Request, RequestEnvelope, ResponseEnvelope, ToolDescriptor,
};
use runtime_transport::Client;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const NORMAL_WORKERS: usize = 4;
const CONTROL_WORKERS: usize = 2;
const OUTBOX_LIMIT: usize = 64;
const ARCHIVE_LIMIT: usize = 64;
const ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const FLUSH_BATCH: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Normal,
    Control,
}
fn lane(request: &Request) -> Lane {
    match request {
        Request::Cancel { .. } | Request::GetOperation { .. } => Lane::Control,
        Request::Execute { tool, .. }
            if tool == "operation.cancel" || tool == "operation.status" =>
        {
            Lane::Control
        }
        _ => Lane::Normal,
    }
}
fn outbox_count(outbox: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(outbox)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|ext| ext == "json") {
            ensure!(
                entry.file_type()?.is_file() && !entry.file_type()?.is_symlink(),
                "Outbox contém entrada inválida"
            );
            count += 1;
        }
    }
    Ok(count)
}

pub async fn catalog(client: &Client) -> Result<(uuid::Uuid, Vec<ToolDescriptor>)> {
    let reply = client
        .request(
            &RequestEnvelope {
                meta: Metadata::new(None, 10_000),
                payload: Request::Catalog,
            },
            |_| {},
        )
        .await?;
    let data = reply.payload.map_err(|e| anyhow::anyhow!(e.message))?;
    let workspace = data["workspace_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Catálogo sem workspace"))?
        .parse()?;
    Ok((workspace, serde_json::from_value(data["tools"].clone())?))
}

pub async fn execute(
    client: &Client,
    command: RemoteCommand,
    workspace: uuid::Uuid,
) -> RemoteResult {
    let request = command.envelope;
    let allowed = request.meta.workspace_id == Some(workspace)
        && matches!(
            request.payload,
            Request::Catalog
                | Request::Heartbeat
                | Request::Execute { .. }
                | Request::Cancel { .. }
                | Request::GetOperation { .. }
        );
    let result = if !allowed {
        ResponseEnvelope {
            meta: request.meta.clone(),
            payload: Err(ErrorEnvelope::new(
                ErrorCode::Unauthorized,
                "Comando fora do dispositivo/workspace autorizado",
            )),
        }
    } else {
        match client.request(&request, |_| {}).await {
            Ok(reply) => match crate::validate_reply(&request, &reply) {
                Ok(()) => reply,
                Err(_) => ResponseEnvelope {
                    meta: request.meta.clone(),
                    payload: Err(ErrorEnvelope::new(
                        ErrorCode::Disconnected,
                        "Resposta IPC inválida",
                    )),
                },
            },
            Err(_) => ResponseEnvelope {
                meta: request.meta.clone(),
                payload: Err(ErrorEnvelope::new(
                    ErrorCode::Disconnected,
                    "Daemon indisponível; consulte o status da operação antes de repetir",
                )),
            },
        }
    };
    let fallback = result.meta.clone();
    let result = crate::bounded_result(result).unwrap_or_else(|_| ResponseEnvelope {
        meta: fallback,
        payload: Err(ErrorEnvelope::new(
            ErrorCode::Tool,
            "Não foi possível serializar o resultado remoto",
        )),
    });
    RemoteResult {
        request_id: command.request_id,
        envelope: result,
    }
}

fn save_result(outbox: &Path, result: &RemoteResult) -> Result<()> {
    let path = outbox.join(format!("{}.json", result.request_id));
    let bytes = serde_json::to_vec(result)?;
    ensure!(
        bytes.len() <= runtime_protocol::MAX_FRAME + 4096,
        "Resultado excede limite local"
    );
    if path.exists() {
        ensure!(
            std::fs::read(&path)? == bytes,
            "Resultado anterior diferente para o mesmo request_id"
        );
        return Ok(());
    }
    ensure!(
        outbox_count(outbox)? < OUTBOX_LIMIT,
        "Outbox cheia; conexão precisa ser recuperada"
    );
    profile::write_private(&path, &bytes)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ArchivedResult {
    version: u16,
    reason: String,
    http_status: u16,
    archived_at: u64,
    result: RemoteResult,
}
fn archive_result(outbox: &Path, result: &RemoteResult, status: u16, reason: &str) -> Result<bool> {
    let archive = outbox
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Outbox sem diretório privado"))?
        .join("outbox-archive");
    runtime_transport::private_directory(&archive)?;
    let destination = archive.join(format!("{}.json", result.request_id));
    if destination.exists() {
        let metadata = std::fs::symlink_metadata(&destination)?;
        ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= (runtime_protocol::MAX_FRAME + 8192) as u64,
            "Arquivo de auditoria inválido"
        );
        let existing: ArchivedResult = serde_json::from_slice(&std::fs::read(&destination)?)?;
        ensure!(
            existing.version == 1
                && serde_json::to_vec(&existing.result)? == serde_json::to_vec(result)?,
            "Auditoria anterior diferente para o mesmo request_id"
        );
        return Ok(true);
    }
    let record = ArchivedResult {
        version: 1,
        reason: reason.into(),
        http_status: status,
        archived_at: runtime_protocol::now_ms(),
        result: result.clone(),
    };
    let bytes = serde_json::to_vec(&record)?;
    let mut count = 0usize;
    let mut stored = 0u64;
    for entry in std::fs::read_dir(&archive)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        ensure!(
            entry.file_type()?.is_file() && !entry.file_type()?.is_symlink(),
            "Arquivo de auditoria contém entrada inválida"
        );
        count += 1;
        stored = stored.saturating_add(metadata.len());
    }
    // Never evict the only audit record automatically. At capacity, retain the original outbox.
    if count >= ARCHIVE_LIMIT || stored.saturating_add(bytes.len() as u64) > ARCHIVE_BYTES {
        return Ok(false);
    }
    profile::write_private(&destination, &bytes)?;
    Ok(true)
}

async fn flush(service: &ServiceClient, outbox: &Path, cursor: &AtomicUsize) -> Result<()> {
    let mut entries = std::fs::read_dir(outbox)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.retain(|entry| entry.path().extension().is_some_and(|ext| ext == "json"));
    entries.sort_by_key(|entry| entry.file_name());
    if entries.is_empty() {
        return Ok(());
    }
    let start = cursor.fetch_add(FLUSH_BATCH, Ordering::Relaxed) % entries.len();
    entries.rotate_left(start);
    let mut attempted = 0;
    for entry in entries {
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        ensure!(
            entry.file_type()?.is_file() && !entry.file_type()?.is_symlink(),
            "Outbox contém entrada inválida"
        );
        ensure!(
            entry.metadata()?.len() <= (runtime_protocol::MAX_FRAME + 4096) as u64,
            "Outbox excede limite"
        );
        let result: RemoteResult = serde_json::from_slice(&std::fs::read(entry.path())?)?;
        if attempted >= FLUSH_BATCH {
            break;
        }
        attempted += 1;
        match service.result(&result).await {
            Ok(()) => std::fs::remove_file(entry.path())?,
            Err(error) => {
                if let Some(http) = error.downcast_ref::<ServiceHttpError>() {
                    if http.is_revoked() {
                        return Err(error);
                    }
                    if let Some(reason) = http.permanent_result_reason() {
                        match archive_result(outbox, &result, http.status.as_u16(), reason) {
                            Ok(true) => {
                                std::fs::remove_file(entry.path())?;
                                tracing::warn!(request_id=%result.request_id,status=http.status.as_u16(),"Resultado sem destino remoto preservado no arquivo privado de auditoria");
                            }
                            Ok(false) => {
                                tracing::warn!(request_id=%result.request_id,"Arquivo privado de auditoria cheio; resultado mantido na outbox, exporte evidências para liberar espaço")
                            }
                            Err(_) => {
                                tracing::warn!(request_id=%result.request_id,"Não foi possível arquivar resultado; evidência mantida na outbox")
                            }
                        }
                        continue;
                    }
                }
                // A failing result endpoint must not prevent presence, polling or other deliveries.
                tracing::warn!(request_id=%result.request_id,"Entrega temporariamente indisponível; resultado privado preservado para retry");
            }
        }
    }
    Ok(())
}

pub async fn run(
    profile: Profile,
    client: Client,
    private_root: PathBuf,
    stop: CancellationToken,
) -> Result<()> {
    let service = ServiceClient::new(
        &profile.server,
        profile.allow_http_loopback,
        Some(profile.device_token),
    )?;
    let (workspace, tools) = catalog(&client).await?;
    let outbox = private_root.join("outbox");
    runtime_transport::private_directory(&outbox)?;
    let mut workers = JoinSet::new();
    let mut active = HashMap::new();
    let mut failures = 0u32;
    let flush_cursor = AtomicUsize::new(0);
    let mut heartbeat_at = tokio::time::Instant::now() - Duration::from_secs(60);
    'agent: loop {
        while let Some(done) = workers.try_join_next() {
            let result: RemoteResult = done??;
            active.remove(&result.request_id);
            save_result(&outbox, &result)?;
        }
        if stop.is_cancelled() {
            break;
        }
        // Finish startup/backlog flushing before polling. Worker completions can then flush
        // during a long poll without racing another in-flight flush of the same private file.
        let flush_result = tokio::select! {_=stop.cancelled()=>break 'agent,response=flush(&service,&outbox,&flush_cursor)=>response};
        let attempt = async {
            flush_result?;
            if heartbeat_at.elapsed() >= Duration::from_secs(15) {
                service.heartbeat(workspace, &tools).await?;
                heartbeat_at = tokio::time::Instant::now();
            }
            service.poll().await
        };
        tokio::pin!(attempt);
        let response = loop {
            tokio::select! {
                _=stop.cancelled()=>break 'agent,
                reply=&mut attempt=>break reply,
                done=workers.join_next(), if !workers.is_empty()=>{
                    let result: RemoteResult = done.ok_or_else(||anyhow::anyhow!("Worker sem resultado"))???;
                    active.remove(&result.request_id);
                    save_result(&outbox,&result)?;
                    let delivered=tokio::select!{_=stop.cancelled()=>break 'agent,response=flush(&service,&outbox,&flush_cursor)=>response};
                    if let Err(error)=delivered {break Err(error);}
                }
            }
        };
        match response {
            Ok(response) => {
                failures = 0;
                let pending = outbox_count(&outbox)?;
                for command in response.commands {
                    if active.contains_key(&command.request_id)
                        || outbox.join(format!("{}.json", command.request_id)).exists()
                    {
                        continue;
                    }
                    let lane = lane(&command.envelope.payload);
                    let limit = if lane == Lane::Control {
                        CONTROL_WORKERS
                    } else {
                        NORMAL_WORKERS
                    };
                    if active
                        .values()
                        .filter(|&&active_lane| active_lane == lane)
                        .count()
                        >= limit
                    {
                        continue;
                    }
                    // Ordinary work also leaves two durable slots for the independent control lane.
                    let result_capacity = if lane == Lane::Control {
                        OUTBOX_LIMIT
                    } else {
                        OUTBOX_LIMIT - CONTROL_WORKERS
                    };
                    if pending.saturating_add(active.len()) >= result_capacity {
                        continue;
                    }
                    active.insert(command.request_id, lane);
                    let local = client.clone();
                    workers.spawn(async move {
                        Ok::<_, anyhow::Error>(execute(&local, command, workspace).await)
                    });
                }
            }
            Err(error) => {
                if error
                    .downcast_ref::<ServiceHttpError>()
                    .is_some_and(ServiceHttpError::is_revoked)
                {
                    stop.cancel();
                    workers.abort_all();
                    return Err(error);
                }
                failures = failures.saturating_add(1);
                tracing::warn!(
                    attempt = failures,
                    "Conexão indisponível; reconectando com resultados preservados"
                );
                let seconds = (1u64 << failures.min(5)).min(30);
                tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_secs(seconds))=>{} }
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    Ok(())
}
