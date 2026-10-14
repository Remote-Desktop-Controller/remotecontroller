//! Receptor de ordens do RDC Maestro (Fase 4/5) no Named Pipe `\\.\pipe\rdc-agent-maestro`.
//!
//! O Maestro despacha o plano aprovado pelo operador como JSON-lines com o schema
//! `rdc.maestro.order.v1`. Este módulo converte cada passo do plano em um
//! `Request::Execute` e o encaminha para `Kernel::dispatch`, ou seja, a ordem percorre
//! exatamente a mesma trilha de política, checkpoint e telemetria do IPC principal —
//! nada é executado fora do executor do kernel.
//!
//! Ao final, um `Ack` (`rdc.maestro.ack.v1`) é escrito de volta no pipe para o Maestro
//! saber o que foi fisicamente executado.

#![allow(dead_code, unused_imports)]

use anyhow::{Context, Result};
use runtime_infrastructure::{
    kernel::{Kernel, wire_error},
    observer::{AuditEvent, AuditEventType, AuditStatus, TelemetryHub},
};
use runtime_ports::ProgressPort;
use runtime_protocol::{Metadata, Request, RequestEnvelope, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(windows)]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

/// Canal dedicado de ordens físicas do Maestro.
pub const MAESTRO_PIPE: &str = r"\\.\pipe\rdc-agent-maestro";

/// Schema do envelope de ordem aceito neste canal.
pub const ORDER_SCHEMA: &str = "rdc.maestro.order.v1";

/// Tipos de ordem aceitos neste canal.
pub const ORDER_KIND_EXECUTE: &str = "execute_approved_plan";
pub const ORDER_KIND_INSPECT: &str = "inspect_workspace";
pub const ORDER_KIND_AUTO_READ: &str = "auto_read";
pub const ORDER_KIND_ROLLBACK: &str = "rollback_checkpoint";

/// Tipo padrão de ordem aceito neste canal.
pub const ORDER_KIND: &str = ORDER_KIND_EXECUTE;

/// Schema do recibo devolvido ao Maestro.
pub const ACK_SCHEMA: &str = "rdc.maestro.ack.v1";

/// Teto de bytes de uma ordem (o Maestro trunca o payload bem abaixo disso).
pub const MAX_ORDER_BYTES: usize = 1024 * 1024;

/// Teto do recibo: precisa caber em uma única leitura do cliente (buffer de 4 KiB).
pub const MAX_ACK_BYTES: usize = 3_800;

/// Limite de passos por ordem.
pub const MAX_STEPS: usize = 64;

/// Prazo para receber a ordem depois que o cliente conecta no pipe.
pub const ORDER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Ordem despachada pelo Maestro (formato tolerante: campos extras são ignorados).
#[derive(Debug, Clone, Deserialize)]
pub struct MaestroOrder {
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub issued_at: Option<u64>,
    #[serde(default)]
    pub mission_id: Option<String>,
    #[serde(default)]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub context_log: Option<Value>,
    #[serde(default)]
    pub payload_encoding: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// Classifica se a ferramenta é puramente de inspeção / leitura segura.
pub fn is_readonly_tool(tool: &str) -> bool {
    let norm = normalize_tool(tool);
    matches!(
        norm.as_str(),
        "filesystem.read"
            | "filesystem.read_range"
            | "filesystem.list"
            | "filesystem.search"
            | "filesystem.stat"
            | "git.status"
            | "git.diff"
            | "git.show"
            | "context.overview"
            | "code.inspect"
            | "code.symbols"
    )
}

/// Classifica se a ferramenta é de mutação física no disco ou processos.
pub fn is_mutating_tool(tool: &str) -> bool {
    let norm = normalize_tool(tool);
    matches!(
        norm.as_str(),
        "filesystem.write"
            | "filesystem.patch"
            | "filesystem.batch_edit"
            | "process.spawn"
            | "checkpoint.rollback"
    )
}

/// Um passo físico do plano aprovado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub tool: String,
    #[serde(default)]
    pub input: Value,
}

impl PlanStep {
    pub fn is_readonly(&self) -> bool {
        is_readonly_tool(&self.tool)
    }

    pub fn is_mutating(&self) -> bool {
        is_mutating_tool(&self.tool)
    }
}

/// Executa o servidor do canal do Maestro até o cancelamento.
#[cfg(windows)]
pub async fn serve(kernel: Arc<Kernel>, endpoint: &str, stop: CancellationToken) -> Result<()> {
    let mut is_first = true;
    tracing::info!(endpoint = %endpoint, "maestro order channel listening");

    loop {
        if stop.is_cancelled() || kernel.shutdown.is_cancelled() {
            break;
        }

        let server = match runtime_transport::create_private_pipe(endpoint, is_first) {
            Ok(server) => {
                is_first = false;
                server
            }
            Err(error) => {
                tracing::warn!(endpoint = %endpoint, %error, "maestro pipe unavailable; retrying");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };

        tokio::select! {
            _ = stop.cancelled() => break,
            _ = kernel.shutdown.cancelled() => break,
            connected = server.connect() => {
                if let Err(error) = connected {
                    tracing::debug!(%error, "maestro client connection failed");
                    continue;
                }
            }
        }

        let kernel = kernel.clone();
        let stop_client = stop.clone();
        tokio::spawn(async move {
            if let Err(error) = connection(server, kernel, stop_client).await {
                tracing::warn!(%error, "maestro order rejected");
            }
        });
    }

    tracing::info!(endpoint = %endpoint, "maestro order channel stopped");
    Ok(())
}

/// O canal do Maestro usa Named Pipe do Windows; nas demais plataformas o daemon segue
/// operando pelo IPC principal sem este canal.
#[cfg(not(windows))]
pub async fn serve(kernel: Arc<Kernel>, endpoint: &str, _stop: CancellationToken) -> Result<()> {
    let _ = kernel;
    tracing::info!(
        endpoint = %endpoint,
        "maestro order channel requires Windows Named Pipes; skipped on this platform"
    );
    Ok(())
}

/// Atende uma conexão: lê uma ordem, executa os passos e devolve o recibo.
#[cfg(windows)]
async fn connection(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    kernel: Arc<Kernel>,
    stop: CancellationToken,
) -> Result<()> {
    let mut line = String::new();
    {
        let reader = BufReader::new(&mut server);
        // Leitura limitada: um cliente não consegue fazer o daemon alocar sem teto.
        let mut limited = reader.take(MAX_ORDER_BYTES as u64);
        let read = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            read = tokio::time::timeout(ORDER_READ_TIMEOUT, limited.read_line(&mut line)) => read,
        };
        match read {
            Ok(Ok(0)) => return Ok(()),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return Err(error).context("read maestro order"),
            Err(_) => {
                anyhow::bail!("maestro order deadline expired");
            }
        }
    }

    // Atingir o teto de leitura significa que a ordem foi truncada no meio.
    if line.len() >= MAX_ORDER_BYTES {
        return write_ack(
            &mut server,
            rejected_ack(None, "ordem excede o limite de tamanho permitido"),
        )
        .await;
    }
    let order = line.trim();
    if order.is_empty() {
        return Ok(());
    }

    let ack = process_order(&kernel, order).await;
    let mission = ack
        .get("mission_id")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let status = ack
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    tracing::info!(mission, status, "maestro order processed");

    write_ack(&mut server, ack).await
}

/// Escreve o recibo (uma linha JSON) e libera o buffer do pipe.
#[cfg(windows)]
async fn write_ack(
    server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
    ack: Value,
) -> Result<()> {
    // Buffer único: reduz a chance de o cliente observar o recibo pela metade.
    let mut payload = encode_ack(&ack).into_bytes();
    payload.push(b'\n');
    server
        .write_all(&payload)
        .await
        .context("write maestro ack")?;
    server.flush().await.context("flush maestro ack")?;
    Ok(())
}

/// Converte a ordem recebida em passos e executa cada um pelo kernel.
pub async fn process_order(kernel: &Arc<Kernel>, raw: &str) -> Value {
    let order: MaestroOrder = match serde_json::from_str(raw) {
        Ok(order) => order,
        Err(error) => {
            tracing::warn!(%error, "malformed maestro order");
            return rejected_ack(None, &format!("ordem ilegível: {error}"));
        }
    };

    if let Err(reason) = validate_header(&order) {
        return rejected_ack(order.mission_id.clone(), &reason);
    }

    let steps = match plan_steps(&order) {
        Ok(steps) => steps,
        Err(reason) => {
            tracing::warn!(
                mission = order.mission_id.as_deref().unwrap_or("-"),
                reason,
                "maestro plan refused"
            );
            return rejected_ack(order.mission_id.clone(), &reason);
        }
    };

    let started_at = now_ms();
    let mission = order.mission_id.clone().unwrap_or_else(|| "-".to_string());
    tracing::info!(
        mission,
        steps = steps.len(),
        encoding = order.payload_encoding.as_deref().unwrap_or("desconhecido"),
        issued_at = order.issued_at.unwrap_or_default(),
        "executing approved maestro plan"
    );

    let context_log = order
        .context_log
        .clone()
        .or_else(|| order.payload.get("context_log").cloned());
    if let Some(ref cl) = context_log {
        let intent = cl.get("intent").and_then(Value::as_str).unwrap_or("-");
        let files = cl.get("files_inspected");
        let verification = cl
            .get("verification_plan")
            .and_then(Value::as_str)
            .unwrap_or("-");
        tracing::info!(
            mission = %mission,
            intent,
            ?files,
            verification,
            "compliance context_log received"
        );
    }

    let is_expanded_read = matches!(
        order.kind.as_deref(),
        Some(ORDER_KIND_INSPECT) | Some(ORDER_KIND_AUTO_READ)
    );
    let max_output_preview = if is_expanded_read { 32 * 1024 } else { 2000 };

    let mut executed = Vec::with_capacity(steps.len());
    let mut failure: Option<Value> = None;
    let mut last_checkpoint_id: Option<String> = None;
    let mut last_restored_files: Option<Value> = None;

    for (index, step) in steps.iter().enumerate() {
        let started = Instant::now();
        let mut actual_step = step.clone();

        // 1.3: Se o passo for filesystem.patch, checa se o trecho SEARCH existe no arquivo alvo
        if actual_step.tool == "filesystem.patch" {
            let target_path = actual_step
                .input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("");
            let find_str = actual_step
                .input
                .get("find")
                .and_then(Value::as_str)
                .unwrap_or("");

            if find_str.trim().is_empty() {
                let elapsed = started.elapsed().as_millis() as u64;
                let wire_msg = format!(
                    "SearchBlockNotFound: trecho SEARCH está vazio para o arquivo '{target_path}'"
                );
                let mut audit_details = json!({
                    "source": "rdc-maestro",
                    "mission_id": mission,
                    "step": index,
                    "error": wire_msg,
                });
                if let Some(ref cl) = context_log {
                    audit_details["context_log"] = cl.clone();
                }
                kernel.telemetry.broadcast(AuditEvent {
                    timestamp: now_ms(),
                    event_type: event_type_for(&actual_step.tool),
                    target: actual_step.tool.clone(),
                    diff: None,
                    execution_time_ms: elapsed,
                    status: AuditStatus::Failed,
                    details: Some(audit_details),
                });
                tracing::warn!(mission, step = index, tool = %actual_step.tool, error = %wire_msg, "maestro search block is empty");
                executed.push(json!({
                    "index": index,
                    "tool": actual_step.tool,
                    "status": "failed",
                    "is_readonly": false,
                    "execution_time_ms": elapsed,
                }));
                failure = Some(json!({
                    "index": index,
                    "tool": actual_step.tool,
                    "error": { "code": "SearchBlockNotFound", "message": wire_msg, "retryable": false },
                }));
                break;
            }

            let read_envelope = RequestEnvelope {
                meta: Metadata::new(
                    Some(kernel.workspace.id.as_uuid()),
                    kernel.config.operation_timeout_ms,
                ),
                payload: Request::Execute {
                    tool: "filesystem.read".to_string(),
                    input: json!({ "path": target_path }),
                },
            };
            let read_progress = Arc::new(StepProgress {
                mission: mission.clone(),
                step: index,
                hub: kernel.telemetry.clone(),
            });

            match kernel.dispatch(read_envelope, read_progress).await {
                Ok(read_val) => {
                    let file_content = read_val
                        .get("result")
                        .and_then(|r| r.get("content"))
                        .and_then(Value::as_str)
                        .or_else(|| read_val.get("content").and_then(Value::as_str))
                        .unwrap_or("");
                    if !file_content.contains(find_str) {
                        // Verifica se apenas as quebras de linha diferem (CRLF vs LF)
                        let crlf_find = find_str.replace("\r\n", "\n").replace('\n', "\r\n");
                        let lf_find = find_str.replace("\r\n", "\n");
                        if file_content.contains(&crlf_find) {
                            actual_step.input["find"] = json!(crlf_find);
                            if let Some(rep) = actual_step.input.get("replace").and_then(Value::as_str) {
                                actual_step.input["replace"] = json!(rep.replace("\r\n", "\n").replace('\n', "\r\n"));
                            }
                        } else if file_content.contains(&lf_find) {
                            actual_step.input["find"] = json!(lf_find);
                            if let Some(rep) = actual_step.input.get("replace").and_then(Value::as_str) {
                                actual_step.input["replace"] = json!(rep.replace("\r\n", "\n"));
                            }
                        } else {
                            let elapsed = started.elapsed().as_millis() as u64;
                            let wire_msg = format!(
                                "SearchBlockNotFound: trecho SEARCH não encontrado no arquivo '{target_path}'"
                            );
                            let mut audit_details = json!({
                                "source": "rdc-maestro",
                                "mission_id": mission,
                                "step": index,
                                "error": wire_msg,
                            });
                            if let Some(ref cl) = context_log {
                                audit_details["context_log"] = cl.clone();
                            }
                            kernel.telemetry.broadcast(AuditEvent {
                                timestamp: now_ms(),
                                event_type: event_type_for(&actual_step.tool),
                                target: actual_step.tool.clone(),
                                diff: None,
                                execution_time_ms: elapsed,
                                status: AuditStatus::Failed,
                                details: Some(audit_details),
                            });
                            tracing::warn!(mission, step = index, tool = %actual_step.tool, error = %wire_msg, "maestro search block missing");
                            executed.push(json!({
                                "index": index,
                                "tool": actual_step.tool,
                                "status": "failed",
                                "is_readonly": false,
                                "execution_time_ms": elapsed,
                            }));
                            failure = Some(json!({
                                "index": index,
                                "tool": actual_step.tool,
                                "error": { "code": "SearchBlockNotFound", "message": wire_msg, "retryable": false },
                            }));
                            break;
                        }
                    }
                }
                Err(error) => {
                    let elapsed = started.elapsed().as_millis() as u64;
                    let wire = wire_error(error);
                    let mut audit_details = json!({
                        "source": "rdc-maestro",
                        "mission_id": mission,
                        "step": index,
                        "error": wire.message,
                    });
                    if let Some(ref cl) = context_log {
                        audit_details["context_log"] = cl.clone();
                    }
                    kernel.telemetry.broadcast(AuditEvent {
                        timestamp: now_ms(),
                        event_type: event_type_for(&actual_step.tool),
                        target: actual_step.tool.clone(),
                        diff: None,
                        execution_time_ms: elapsed,
                        status: AuditStatus::Failed,
                        details: Some(audit_details),
                    });
                    tracing::warn!(mission, step = index, tool = %actual_step.tool, error = %wire.message, "maestro patch pre-read failed");
                    executed.push(json!({
                        "index": index,
                        "tool": actual_step.tool,
                        "status": "failed",
                        "is_readonly": false,
                        "execution_time_ms": elapsed,
                    }));
                    failure = Some(json!({
                        "index": index,
                        "tool": actual_step.tool,
                        "error": { "code": format!("{:?}", wire.code), "message": wire.message, "retryable": wire.retryable },
                    }));
                    break;
                }
            }
        }

        let metadata = Metadata::new(
            Some(kernel.workspace.id.as_uuid()),
            kernel.config.operation_timeout_ms,
        );
        let envelope = RequestEnvelope {
            meta: metadata,
            payload: Request::Execute {
                tool: actual_step.tool.clone(),
                input: actual_step.input.clone(),
            },
        };
        let progress = Arc::new(StepProgress {
            mission: mission.clone(),
            step: index,
            hub: kernel.telemetry.clone(),
        });

        match kernel.dispatch(envelope, progress).await {
            Ok(value) => {
                let elapsed = started.elapsed().as_millis() as u64;
                let preview = build_output_preview(&value, max_output_preview);
                let is_ro = is_readonly_tool(&actual_step.tool);

                let mut details = json!({
                    "source": "rdc-maestro",
                    "mission_id": mission,
                    "step": index,
                });
                if let Some(ref cl) = context_log {
                    details["context_log"] = cl.clone();
                }

                kernel.telemetry.broadcast(AuditEvent {
                    timestamp: now_ms(),
                    event_type: event_type_for(&actual_step.tool),
                    target: actual_step.tool.clone(),
                    diff: None,
                    execution_time_ms: elapsed,
                    status: AuditStatus::Success,
                    details: Some(details),
                });
                tracing::info!(mission, step = index, tool = %actual_step.tool, elapsed_ms = elapsed, "maestro step executed");

                let mut step_exec = json!({
                    "index": index,
                    "tool": actual_step.tool,
                    "status": "ok",
                    "is_readonly": is_ro,
                    "execution_time_ms": elapsed,
                    "output_preview": preview,
                    "result": value.clone(),
                });

                let inner_result = value.get("result").unwrap_or(&value);
                if let Some(cp_id) = inner_result.get("checkpoint_id").and_then(Value::as_str) {
                    step_exec["checkpoint_id"] = json!(cp_id);
                    if last_checkpoint_id.is_none() {
                        last_checkpoint_id = Some(cp_id.to_string());
                    }
                }
                if let Some(restored_id) = inner_result.get("restored_checkpoint_id").and_then(Value::as_str) {
                    step_exec["restored_checkpoint_id"] = json!(restored_id);
                }
                if let Some(restored_files) = inner_result.get("restored_files") {
                    step_exec["restored_files"] = restored_files.clone();
                    last_restored_files = Some(restored_files.clone());
                }
                executed.push(step_exec);
            }
            Err(error) => {
                let elapsed = started.elapsed().as_millis() as u64;
                let wire = wire_error(error);
                let mut details = json!({
                    "source": "rdc-maestro",
                    "mission_id": mission,
                    "step": index,
                    "error": wire.message,
                });
                if let Some(ref cl) = context_log {
                    details["context_log"] = cl.clone();
                }

                kernel.telemetry.broadcast(AuditEvent {
                    timestamp: now_ms(),
                    event_type: event_type_for(&actual_step.tool),
                    target: actual_step.tool.clone(),
                    diff: None,
                    execution_time_ms: elapsed,
                    status: AuditStatus::Failed,
                    details: Some(details),
                });
                tracing::warn!(mission, step = index, tool = %actual_step.tool, error = %wire.message, "maestro step denied or failed");
                executed.push(json!({
                    "index": index,
                    "tool": actual_step.tool,
                    "status": "failed",
                    "is_readonly": is_readonly_tool(&actual_step.tool),
                    "execution_time_ms": elapsed,
                }));
                failure = Some(json!({
                    "index": index,
                    "tool": actual_step.tool,
                    "error": { "code": format!("{:?}", wire.code), "message": wire.message, "retryable": wire.retryable },
                }));
                break;
            }
        }
    }

    let status = if failure.is_some() {
        "failed"
    } else {
        "completed"
    };
    let mut ack = json!({
        "schema": ACK_SCHEMA,
        "order_schema": order.schema.clone().unwrap_or_else(|| ORDER_SCHEMA.to_string()),
        "order_kind": order.kind.clone().unwrap_or_else(|| ORDER_KIND.to_string()),
        "source": order.source.clone().unwrap_or_else(|| "rdc-maestro".to_string()),
        "mission_id": order.mission_id.clone(),
        "status": status,
        "steps_total": steps.len(),
        "steps_executed": executed.len(),
        "order_issued_at": order.issued_at,
        "started_at": started_at,
        "finished_at": now_ms(),
        "steps": executed,
        "truncated": false,
    });
    if let Some(ref cp_id) = last_checkpoint_id {
        ack["checkpoint_id"] = json!(cp_id);
    }
    if let Some(ref rf) = last_restored_files {
        ack["restored_files"] = rf.clone();
    }
    if let Some(ref cl) = context_log {
        ack["context_log"] = cl.clone();
    }
    if let Some(failure) = failure {
        ack["failure"] = failure;
    }
    ack
}

/// Constrói um resumo truncado seguro do resultado da ferramenta para o ack.
pub fn build_output_preview(value: &Value, max: usize) -> String {
    let target = value.get("result").unwrap_or(value);
    let raw = if let Some(content) = target.get("content").and_then(Value::as_str) {
        content.to_string()
    } else if let Some(paths) = target.get("paths") {
        if let Some(arr) = paths.as_array() {
            arr.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            serde_json::to_string(paths).unwrap_or_default()
        }
    } else if let Some(s) = target.as_str() {
        s.to_string()
    } else {
        serde_json::to_string(target).unwrap_or_default()
    };
    if raw.len() > max {
        format!("{}... [truncated]", &raw[..max])
    } else {
        raw
    }
}

/// Valida o cabeçalho da ordem antes de qualquer execução física.
#[allow(clippy::collapsible_if)]
fn validate_header(order: &MaestroOrder) -> Result<(), String> {
    if let Some(schema) = order.schema.as_deref() {
        if schema != ORDER_SCHEMA {
            return Err(format!("schema de ordem não suportado: {schema}"));
        }
    }
    if let Some(kind) = order.kind.as_deref() {
        if kind != ORDER_KIND_EXECUTE
            && kind != ORDER_KIND_INSPECT
            && kind != ORDER_KIND_AUTO_READ
            && kind != ORDER_KIND_ROLLBACK
        {
            return Err(format!("tipo de ordem não suportado: {kind}"));
        }
    }
    Ok(())
}

/// Recibo de recusa (schema inválido, plano ilegível ou fora de política).
fn rejected_ack(mission_id: Option<String>, reason: &str) -> Value {
    json!({
        "schema": ACK_SCHEMA,
        "mission_id": mission_id,
        "status": "rejected",
        "finished_at": now_ms(),
        "steps_total": 0,
        "steps_executed": 0,
        "steps": Vec::<Value>::new(),
        "truncated": false,
        "failure": { "index": null, "tool": null, "error": { "code": "InvalidRequest", "message": reason, "retryable": false } },
    })
}

/// Serializa o recibo respeitando o teto de bytes do cliente.
///
/// Se necessário, os resultados dos passos são omitidos e, em último caso, apenas os
/// últimos passos são relatados — sempre sinalizando `truncated: true`.
pub fn encode_ack(ack: &Value) -> String {
    let mut current = ack.clone();
    let is_expanded = matches!(
        current.get("order_kind").and_then(Value::as_str),
        Some(ORDER_KIND_INSPECT) | Some(ORDER_KIND_AUTO_READ)
    );
    let max_bytes = if is_expanded { 64 * 1024 } else { MAX_ACK_BYTES };

    let mut serialized = serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string());
    if serialized.len() <= max_bytes {
        return serialized;
    }

    current["truncated"] = json!(true);
    if let Some(steps) = current.get_mut("steps").and_then(Value::as_array_mut) {
        for step in steps.iter_mut() {
            if let Some(object) = step.as_object_mut() {
                object.remove("result");
            }
        }
    }
    serialized = serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string());
    if serialized.len() <= max_bytes {
        return serialized;
    }

    if let Some(steps) = current.get_mut("steps").and_then(Value::as_array_mut) {
        let keep = steps.len().min(8);
        let tail = steps.split_off(steps.len() - keep);
        *steps = tail;
    }
    serialized = serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string());
    if serialized.len() <= max_bytes {
        return serialized;
    }

    // Último recurso: recibo mínimo, sem passos.
    current["steps"] = json!([]);
    serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string())
}

fn event_type_for(tool: &str) -> AuditEventType {
    if tool.starts_with("process.") {
        AuditEventType::CommandExecuted
    } else if tool.starts_with("filesystem.write")
        || tool.starts_with("filesystem.patch")
        || tool.starts_with("filesystem.batch_edit")
    {
        AuditEventType::FileModified
    } else {
        AuditEventType::ToolInvoked
    }
}

/// Progresso de um passo publicado na telemetria e no log do daemon.
struct StepProgress {
    mission: String,
    step: usize,
    hub: Arc<TelemetryHub>,
}

impl ProgressPort for StepProgress {
    fn emit(&self, completed: usize, total: usize, message: &str) {
        tracing::debug!(
            mission = %self.mission,
            step = self.step,
            completed,
            total,
            message,
            "maestro step progress"
        );
        self.hub.broadcast(AuditEvent {
            timestamp: now_ms(),
            event_type: AuditEventType::ToolInvoked,
            target: format!("maestro.step.{}", self.step),
            diff: None,
            execution_time_ms: 0,
            status: AuditStatus::Success,
            details: Some(json!({
                "source": "rdc-maestro",
                "mission_id": self.mission,
                "completed": completed,
                "total": total,
                "message": message,
            })),
        });
    }
}

/// Extrai os passos executáveis do payload da ordem aprovada.
pub fn plan_steps(order: &MaestroOrder) -> Result<Vec<PlanStep>, String> {
    if order.kind.as_deref() == Some(ORDER_KIND_ROLLBACK) {
        let cp_id = order
            .checkpoint_id
            .clone()
            .or_else(|| {
                order
                    .payload
                    .get("checkpoint_id")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .or_else(|| {
                if let Value::String(s) = &order.payload {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('{') {
                        Some(trimmed.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

        if let Some(cp_id) = cp_id {
            return Ok(vec![PlanStep {
                tool: "checkpoint.rollback".to_string(),
                input: json!({ "checkpoint_id": cp_id }),
            }]);
        }
    }

    let steps = collect_steps(&order.payload)?;
    if steps.is_empty() {
        return Err("plano aprovado sem passos executáveis".to_string());
    }
    if steps.len() > MAX_STEPS {
        return Err(format!(
            "plano com {} passos excede o limite de {MAX_STEPS}",
            steps.len()
        ));
    }
    Ok(steps)
}

fn collect_steps(payload: &Value) -> Result<Vec<PlanStep>, String> {
    match payload {
        Value::Array(items) => items.iter().map(step_from_value).collect(),
        Value::Object(map) => {
            for key in ["steps", "plan", "actions", "commands"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return items.iter().map(step_from_value).collect();
                }
            }
            step_from_value(payload).map(|step| vec![step])
        }
        Value::String(text) => {
            let sr_steps = extract_search_replace_steps(text);
            if !sr_steps.is_empty() {
                return Ok(sr_steps);
            }
            match extract_json_block(text) {
                Some(embedded) => collect_steps(&embedded),
                None => Err(
                    "plano em texto livre sem bloco JSON estruturado: inclua um objeto {\"steps\":[{\"tool\":\"...\",\"input\":{...}}]} ou bloco SEARCH/REPLACE para execução física"
                        .to_string(),
                ),
            }
        }
        _ => Err("payload de ordem não suportado (esperado objeto, lista ou texto)".to_string()),
    }
}

/// Extrai blocos cirúrgicos no formato SEARCH/REPLACE:
/// ```text
/// <<<<<<< SEARCH: caminho/do/arquivo.ext
/// trecho exato antigo
/// =======
/// trecho novo atualizado
/// >>>>>>> REPLACE
/// ```
pub fn extract_search_replace_steps(text: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("<<<<<<< SEARCH") {
            let mut path = trimmed
                .strip_prefix("<<<<<<< SEARCH:")
                .or_else(|| trimmed.strip_prefix("<<<<<<< SEARCH :"))
                .or_else(|| trimmed.strip_prefix("<<<<<<< SEARCH"))
                .unwrap_or("")
                .trim()
                .trim_matches(':')
                .trim()
                .trim_matches('`')
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();

            i += 1;
            let mut search_lines = Vec::new();
            while i < lines.len() && lines[i].trim() != "=======" {
                search_lines.push(lines[i]);
                i += 1;
            }
            if i >= lines.len() {
                break;
            }
            // Pula a linha "======="
            i += 1;
            let mut replace_lines = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with(">>>>>>> REPLACE") {
                replace_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() && lines[i].trim().starts_with(">>>>>>> REPLACE") {
                i += 1;
            }
            if path.is_empty() && !search_lines.is_empty() {
                let first = search_lines[0].trim();
                if let Some(p) = first
                    .strip_prefix("file:")
                    .or_else(|| first.strip_prefix("// path:"))
                    .or_else(|| first.strip_prefix("# path:"))
                {
                    path = p
                        .trim()
                        .trim_matches(':')
                        .trim()
                        .trim_matches('`')
                        .trim_matches('"')
                        .trim_matches('\'')
                        .trim()
                        .to_string();
                    search_lines.remove(0);
                }
            }
            if !path.is_empty() {
                steps.push(PlanStep {
                    tool: "filesystem.patch".to_string(),
                    input: json!({
                        "path": path,
                        "find": search_lines.join("\n"),
                        "replace": replace_lines.join("\n"),
                    }),
                });
            }
        } else {
            i += 1;
        }
    }
    steps
}

fn step_from_value(value: &Value) -> Result<PlanStep, String> {
    let map = value
        .as_object()
        .ok_or_else(|| "cada passo do plano deve ser um objeto JSON".to_string())?;

    if let Some(tool) = map.get("tool").and_then(Value::as_str) {
        let mut input = map
            .get("input")
            .or_else(|| map.get("args"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let norm_tool = normalize_tool(tool);
        if norm_tool == "filesystem.patch" && input.get("find").is_none() {
            if let Some(search) = input.get("search") {
                input["find"] = search.clone();
            }
        }
        if norm_tool == "checkpoint.rollback" && input.get("checkpoint_id").is_none() {
            if let Some(id) = input.get("id") {
                input["checkpoint_id"] = id.clone();
            }
        }
        return Ok(PlanStep {
            tool: norm_tool,
            input,
        });
    }

    if let Some(command) = map
        .get("command")
        .or_else(|| map.get("cmd"))
        .or_else(|| map.get("shell"))
        .and_then(Value::as_str)
    {
        return command_step(command);
    }

    if let Some(program) = map.get("program").and_then(Value::as_str) {
        let mut input = json!({
            "program": program,
            "args": map.get("args").cloned().unwrap_or_else(|| json!([])),
        });
        if let Some(cwd) = map.get("cwd") {
            input["cwd"] = cwd.clone();
        }
        return Ok(PlanStep {
            tool: "process.spawn".to_string(),
            input,
        });
    }

    if map.contains_key("path") {
        let tool = if map.contains_key("content") {
            "filesystem.write"
        } else {
            "filesystem.read"
        };
        return Ok(PlanStep {
            tool: tool.to_string(),
            input: value.clone(),
        });
    }

    Err("passo sem 'tool', 'command', 'program' ou 'path' reconhecível".to_string())
}

fn command_step(command: &str) -> Result<PlanStep, String> {
    let parts = split_command_line(command);
    let (program, args) = parts
        .split_first()
        .ok_or_else(|| "comando vazio no passo do plano".to_string())?;
    Ok(PlanStep {
        tool: "process.spawn".to_string(),
        input: json!({ "program": program, "args": args }),
    })
}

/// Normaliza nomes amigáveis (PT-BR e inglês) para os tools do kernel.
pub fn normalize_tool(tool: &str) -> String {
    let trimmed = tool.trim();
    match trimmed.to_lowercase().as_str() {
        "read" | "read_file" | "file_read" | "ler" | "ler_arquivo" | "cat" => "filesystem.read",
        "read_range" | "ler_trecho" => "filesystem.read_range",
        "write" | "write_file" | "file_write" | "escrever" | "criar_arquivo" | "save_file" => {
            "filesystem.write"
        }
        "patch" | "edit" | "replace" | "substituir" | "editar" | "patch_search_replace" => {
            "filesystem.patch"
        }
        "checkpoint.restore" | "checkpoint.rollback" | "rollback" | "rollback_checkpoint" => {
            "checkpoint.rollback"
        }
        "batch_edit" | "edits" => "filesystem.batch_edit",
        "list" | "list_dir" | "ls" | "listar" | "listar_diretorio" => "filesystem.list",
        "search" | "grep" | "buscar" => "filesystem.search",
        "stat" | "info" | "stat_file" => "filesystem.stat",
        "run" | "exec" | "execute" | "shell" | "terminal" | "command" | "cmd" | "executar"
        | "comando" | "rodar" => "process.spawn",
        _ => trimmed,
    }
    .to_string()
}

/// Divide uma linha de comando respeitando aspas simples/duplas e escapes.
pub fn split_command_line(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in command.trim().chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if quote != Some('\'') => escaped = true,
            '"' | '\'' => {
                if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(character);
                } else {
                    current.push(character);
                }
            }
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Procura um bloco JSON na resposta em texto do consenso (```json ... ``` ou objeto solto).
pub fn extract_json_block(text: &str) -> Option<Value> {
    if let Some(fenced) = extract_fenced_block(text) {
        return Some(fenced);
    }
    extract_balanced(text)
}

fn extract_fenced_block(text: &str) -> Option<Value> {
    let mut remainder = text;
    while let Some(start) = remainder.find("```") {
        let after = &remainder[start + 3..];
        let end = after.find("```")?;
        let body = &after[..end];
        let body = body
            .strip_prefix("json")
            .or_else(|| body.strip_prefix("JSON"))
            .unwrap_or(body)
            .trim();
        if let Ok(value) = serde_json::from_str::<Value>(body) {
            return Some(value);
        }
        remainder = &after[end + 3..];
    }
    None
}

fn extract_balanced(text: &str) -> Option<Value> {
    let bytes = text.as_bytes();
    for (start, byte) in bytes.iter().enumerate() {
        let (open, close) = match byte {
            b'{' => (b'{', b'}'),
            b'[' => (b'[', b']'),
            _ => continue,
        };

        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (offset, current) in bytes[start..].iter().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            match current {
                b'\\' if in_string => escaped = true,
                b'"' => in_string = !in_string,
                c if !in_string && *c == open => depth += 1,
                c if !in_string && *c == close => {
                    depth -= 1;
                    if depth == 0 {
                        let candidate = &text[start..start + offset + 1];
                        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                            return Some(value);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(payload: Value) -> MaestroOrder {
        MaestroOrder {
            schema: Some(ORDER_SCHEMA.to_string()),
            source: Some("rdc-maestro".to_string()),
            kind: Some("execute_approved_plan".to_string()),
            issued_at: Some(0),
            mission_id: Some("m-test".to_string()),
            checkpoint_id: None,
            context_log: None,
            payload_encoding: None,
            payload,
        }
    }

    #[test]
    fn accepts_structured_steps() {
        let steps = plan_steps(&order(json!({
            "steps": [
                {"tool": "filesystem.read", "input": {"path": "README.md"}},
                {"tool": "filesystem.write", "input": {"path": "a.txt", "content": "x"}}
            ]
        })))
        .unwrap();

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].tool, "filesystem.read");
        assert_eq!(steps[1].input["content"], "x");
    }

    #[test]
    fn normalizes_friendly_tool_aliases() {
        assert_eq!(normalize_tool("read_file"), "filesystem.read");
        assert_eq!(normalize_tool("LER_ARQUIVO"), "filesystem.read");
        assert_eq!(normalize_tool("escrever"), "filesystem.write");
        assert_eq!(normalize_tool("terminal"), "process.spawn");
        assert_eq!(normalize_tool("filesystem.stat"), "filesystem.stat");
    }

    #[test]
    fn maps_command_shorthand_to_process_spawn() {
        let steps = plan_steps(&order(json!({
            "steps": [{"command": "cmd /c \"echo ola mundo\""}]
        })))
        .unwrap();

        assert_eq!(steps[0].tool, "process.spawn");
        assert_eq!(steps[0].input["program"], "cmd");
        assert_eq!(steps[0].input["args"][0], "/c");
        assert_eq!(steps[0].input["args"][1], "echo ola mundo");
    }

    #[test]
    fn maps_path_shorthand_to_read_or_write() {
        let read = plan_steps(&order(json!({"steps": [{"path": "src/main.rs"}]}))).unwrap();
        assert_eq!(read[0].tool, "filesystem.read");

        let write = plan_steps(&order(
            json!({"steps": [{"path": "a.txt", "content": "ok"}]}),
        ))
        .unwrap();
        assert_eq!(write[0].tool, "filesystem.write");
    }

    #[test]
    fn extracts_fenced_json_from_consensus_text() {
        let consensus = "# Consenso\n\nPlano aprovado:\n\n```json\n{\"steps\":[{\"tool\":\"filesystem.list\",\"input\":{\"path\":\".\"}}]}\n```\n";
        let steps = plan_steps(&order(json!(consensus))).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "filesystem.list");
    }

    #[test]
    fn extracts_bare_json_object_from_text() {
        let text = "Execute: {\"steps\":[{\"command\":\"cmd /c dir\"}]} conforme aprovado.";
        let steps = plan_steps(&order(json!(text))).unwrap();
        assert_eq!(steps[0].tool, "process.spawn");
    }

    #[test]
    fn refuses_free_text_without_structured_plan() {
        let error = plan_steps(&order(json!("Reiniciar o spooler e limpar a fila"))).unwrap_err();
        assert!(error.contains("bloco JSON estruturado"));
    }

    #[test]
    fn enforces_step_limit() {
        let steps: Vec<Value> = (0..(MAX_STEPS + 1))
            .map(|_| json!({"tool": "filesystem.read", "input": {"path": "a"}}))
            .collect();
        let error = plan_steps(&order(json!({ "steps": steps }))).unwrap_err();
        assert!(error.contains("excede o limite"));
    }

    #[test]
    fn rejects_unknown_step_shape() {
        let error = plan_steps(&order(json!({ "steps": [{ "foo": "bar" }] }))).unwrap_err();
        assert!(error.contains("reconhecível"));
    }

    #[test]
    fn splits_command_lines_with_quotes() {
        assert_eq!(
            split_command_line("cmd /c \"echo a b\" c"),
            vec!["cmd", "/c", "echo a b", "c"]
        );
        assert_eq!(split_command_line("  "), Vec::<String>::new());
    }

    #[test]
    fn ack_is_truncated_under_the_client_buffer() {
        let big = "x".repeat(MAX_ACK_BYTES);
        let ack = json!({
            "schema": ACK_SCHEMA,
            "status": "completed",
            "steps": [{"index": 0, "tool": "filesystem.read", "status": "ok", "result": big}],
            "truncated": false,
        });

        let encoded = encode_ack(&ack);
        assert!(
            encoded.len() <= MAX_ACK_BYTES,
            "ack excedeu o teto: {}",
            encoded.len()
        );
        let parsed: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert!(parsed["steps"][0].get("result").is_none());
    }

    #[test]
    fn rejects_unsupported_schema_without_running_steps() {
        let ack = serde_json::to_value(rejected_ack(
            Some("m-1".to_string()),
            "schema de ordem não suportado: outro",
        ))
        .unwrap();
        assert_eq!(ack["status"], "rejected");
        assert_eq!(ack["steps_executed"], 0);
        assert_eq!(ack["schema"], ACK_SCHEMA);
    }

    #[test]
    fn validates_order_header() {
        assert!(validate_header(&order(json!({ "steps": [] }))).is_ok());

        let mut wrong_kind = order(json!({ "steps": [] }));
        wrong_kind.kind = Some("shutdown_everything".to_string());
        let error = validate_header(&wrong_kind).unwrap_err();
        assert!(error.contains("tipo de ordem não suportado"));

        let mut wrong_schema = order(json!({ "steps": [] }));
        wrong_schema.schema = Some("rdc.maestro.order.v99".to_string());
        let error = validate_header(&wrong_schema).unwrap_err();
        assert!(error.contains("schema de ordem não suportado"));

        // Ordem sem cabeçalho (cliente legado) continua aceita.
        let mut bare = order(json!({ "steps": [] }));
        bare.schema = None;
        bare.kind = None;
        assert!(validate_header(&bare).is_ok());
    }

    #[test]
    fn extracts_search_replace_block_steps() {
        let text = r#"
Vamos aplicar uma melhoria no arquivo:

<<<<<<< SEARCH: src/main.rs
fn old_func() {
    println!("old");
}
=======
fn new_func() {
    println!("new");
}
>>>>>>> REPLACE

Plano concluído com sucesso.
"#;
        let steps = plan_steps(&order(json!(text))).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "filesystem.patch");
        assert_eq!(steps[0].input["path"], "src/main.rs");
        assert_eq!(steps[0].input["find"], "fn old_func() {\n    println!(\"old\");\n}");
        assert_eq!(steps[0].input["replace"], "fn new_func() {\n    println!(\"new\");\n}");
        assert!(steps[0].is_mutating());
        assert!(!steps[0].is_readonly());
    }

    #[test]
    fn handles_patch_search_replace_alias() {
        let steps = plan_steps(&order(json!({
            "steps": [
                {
                    "tool": "patch_search_replace",
                    "input": {
                        "path": "src/lib.rs",
                        "search": "old line",
                        "replace": "new line"
                    }
                }
            ]
        })))
        .unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "filesystem.patch");
        assert_eq!(steps[0].input["find"], "old line");
        assert_eq!(steps[0].input["replace"], "new line");
    }

    #[test]
    fn classifies_readonly_and_mutating_tools() {
        assert!(is_readonly_tool("filesystem.read"));
        assert!(is_readonly_tool("filesystem.read_range"));
        assert!(is_readonly_tool("filesystem.list"));
        assert!(is_readonly_tool("filesystem.search"));
        assert!(is_readonly_tool("filesystem.stat"));
        assert!(is_readonly_tool("git.status"));
        assert!(is_readonly_tool("git.diff"));

        assert!(is_mutating_tool("filesystem.write"));
        assert!(is_mutating_tool("filesystem.patch"));
        assert!(is_mutating_tool("filesystem.batch_edit"));
        assert!(is_mutating_tool("process.spawn"));
        assert!(is_mutating_tool("checkpoint.rollback"));

        assert!(!is_readonly_tool("filesystem.write"));
        assert!(!is_mutating_tool("filesystem.read"));
    }

    #[test]
    fn builds_output_preview_safely() {
        let content_val = json!({"path": "file.txt", "content": "hello world"});
        let preview = build_output_preview(&content_val, 100);
        assert_eq!(preview, "hello world");

        let long_val = json!({"content": "a".repeat(100)});
        let preview_trunc = build_output_preview(&long_val, 10);
        assert!(preview_trunc.contains("... [truncated]"));
        assert!(preview_trunc.starts_with("aaaaaaaaaa"));
    }

    #[test]
    fn accepts_rollback_checkpoint_order() {
        let mut ord = order(json!({}));
        ord.kind = Some("rollback_checkpoint".to_string());
        ord.checkpoint_id = Some("11111111-2222-3333-4444-555555555555".to_string());

        let steps = plan_steps(&ord).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "checkpoint.rollback");
        assert_eq!(
            steps[0].input["checkpoint_id"],
            "11111111-2222-3333-4444-555555555555"
        );
    }

    #[test]
    fn preserves_context_log_in_order() {
        let mut ord = order(json!({
            "steps": [{"tool": "filesystem.read", "input": {"path": "a.txt"}}]
        }));
        ord.context_log = Some(json!({
            "intent": "Inspecionar configurações locais",
            "files_inspected": ["a.txt"],
            "verification_plan": "Verificar se arquivo não está vazio"
        }));

        let cl = ord.context_log.as_ref().unwrap();
        assert_eq!(cl["intent"], "Inspecionar configurações locais");
        assert_eq!(cl["files_inspected"][0], "a.txt");
    }

    #[test]
    fn extracts_search_replace_with_file_prefix_and_quotes() {
        let text = r#"
<<<<<<< SEARCH
file: "src/utils.rs"
let old = 1;
=======
let new = 2;
>>>>>>> REPLACE
"#;
        let steps = plan_steps(&order(json!(text))).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "filesystem.patch");
        assert_eq!(steps[0].input["path"], "src/utils.rs");
        assert_eq!(steps[0].input["find"], "let old = 1;");
        assert_eq!(steps[0].input["replace"], "let new = 2;");
    }

    #[test]
    fn handles_checkpoint_restore_alias_in_step() {
        let steps = plan_steps(&order(json!({
            "steps": [
                {
                    "tool": "checkpoint.restore",
                    "input": {
                        "id": "11111111-2222-3333-4444-555555555555"
                    }
                }
            ]
        })))
        .unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "checkpoint.rollback");
        assert_eq!(
            steps[0].input["checkpoint_id"],
            "11111111-2222-3333-4444-555555555555"
        );
    }
}
