// RDC Maestro - Motor de Orquestração Blackboard (Fase 3) e Ponte IPC com o Daemon (Fase 4)
//
// Responsabilidades deste módulo:
//   1. `MaestroEngine` mantém os canais assíncronos (`tokio::sync::mpsc`) usados pelas
//      WebViews ocultas para devolver o texto lido de cada provedor (blackboard multi-agente).
//   2. `run_debate_cycle` injeta o mesmo prompt simultaneamente em todas as sessões web ocultas
//      (ChatGPT, Claude, Gemini, DeepSeek e Qwen — ver `PROVIDERS`) via `WebviewWindow::eval`, aguarda as respostas no canal
//      e sintetiza o consenso determinístico entre os agentes.
//   3. `execute_approved_plan` atua como cliente do Named Pipe nativo do RDC e despacha a
//      ordem física aprovada pelo operador para o `local-daemon`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{info, warn};

use crate::webview_manager::{provider_names_joined, ProviderConfig, WebviewManager, PROVIDERS};

// --------------------------------------------------------------------------------------
// Constantes de operação
// --------------------------------------------------------------------------------------

/// Capacidade do canal de blackboard (respostas pendentes das WebViews).
pub const REPLY_CHANNEL_CAPACITY: usize = 64;

/// Janela máxima de espera pelas respostas dos agentes antes de consolidar parcialmente.
pub const DEBATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Após o quórum ser atingido, janela extra para aguardar respostas remanescentes
/// (evita travar o ciclo por 2 minutos quando um agente não responde).
pub const POST_QUORUM_GRACE: Duration = Duration::from_secs(25);

/// Número mínimo de agentes que precisam responder para o consenso ser considerado quórum.
pub const REPLY_QUORUM: usize = 2;

/// Similaridade Jaccard mínima entre dois enunciados para considerá-los o mesmo ponto.
pub const CONVERGENCE_THRESHOLD: f64 = 0.34;

/// Similaridade mínima para descartar enunciados redundantes já selecionados.
pub const DEDUPE_THRESHOLD: f64 = 0.60;

/// Limite defensivo do texto aceito de uma WebView (caracteres).
pub const MAX_REPLY_CHARS: usize = 64_000;

/// Limite defensivo do texto de um enunciado individual no consenso.
pub const MAX_STATEMENT_CHARS: usize = 400;

/// Limite de transcrição integral por provedor no documento de consenso.
pub const MAX_TRANSCRIPT_CHARS: usize = 8_000;

/// Máximo de pontos listados em cada seção do consenso.
pub const MAX_CONSENSUS_ITEMS: usize = 12;

/// Teto do histórico de missões mantido em memória pelo motor.
pub const MAX_MISSION_HISTORY: usize = 64;

/// Endpoints candidatos do canal de comando nativo do RDC (ordem de preferência).
///
/// - `\\.\pipe\rdc-agent-maestro`: canal dedicado de ordens físicas do Maestro, a ser
///   exposto pelo `local-daemon` (contraparte da Fase 4). É o alvo preferencial.
/// - `\\.\pipe\rdc-telemetry`: canal de observabilidade já criado pelo daemon
///   (`runtime_infrastructure::observer::server`). Serve como fallback de entrega.
///
/// O envelope enviado é JSON-lines (`rdc.maestro.order.v1`). Enquanto o daemon não
/// consumir o canal dedicado, a ordem é entregue ao pipe disponível e fica registrada
/// no `PlanExecutionReceipt`, sem execução física pelo daemon.
pub const DAEMON_COMMAND_PIPES: &[&str] = &[r"\\.\pipe\rdc-agent-maestro", r"\\.\pipe\rdc-telemetry"];

/// Tentativas de conexão ao pipe antes de desistir (o daemon pode estar inicializando).
pub const DAEMON_CONNECT_ATTEMPTS: usize = 10;

/// Intervalo entre tentativas de conexão ao pipe.
pub const DAEMON_CONNECT_BACKOFF: Duration = Duration::from_millis(150);

/// Tempo máximo de espera por um reconhecimento (ack) do daemon após o despacho.
pub const DAEMON_ACK_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Teto de bytes acumulados ao ler o recibo do daemon.
pub const MAX_ACK_BYTES: usize = 16 * 1024;

/// Esquema do envelope de ordem física enviado ao daemon.
pub const DAEMON_ORDER_SCHEMA: &str = "rdc.maestro.order.v1";

// --------------------------------------------------------------------------------------
// Tipos de domínio
// --------------------------------------------------------------------------------------

/// Nível de severidade dos eventos de log emitidos para a UI (`maestro-log`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    System,
    Info,
    Success,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::System => "system",
            LogLevel::Info => "info",
            LogLevel::Success => "success",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Payload do evento `maestro-log` (compatível com `LogPayload` do frontend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaestroLog {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

/// Resposta devolvida por uma WebView de provedor através do canal de blackboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderReply {
    pub provider: String,
    pub mission_id: Option<String>,
    pub text: String,
    pub ok: bool,
    pub captured_at: u64,
    pub latency_ms: u64,
}

/// Progresso do ciclo de debate, emitido em `maestro-debate-progress`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateProgress {
    pub mission_id: String,
    pub stage: String,
    pub provider: Option<String>,
    pub detail: String,
    pub received: usize,
    pub expected: usize,
    pub timestamp: u64,
}

/// Um ponto (enunciado) do consenso, com os provedores que o sustentam.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusPoint {
    pub text: String,
    pub providers: Vec<String>,
    pub support: usize,
}

/// Registro de acompanhamento de uma missão disparada pela UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionRecord {
    pub mission_id: String,
    pub prompt: String,
    pub status: String,
    pub injected: Vec<String>,
    pub missing: Vec<String>,
    pub received: Vec<String>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

/// Resultado consolidado do ciclo de debate (documento de consenso + auditoria).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateOutcome {
    pub mission_id: String,
    pub prompt: String,
    pub status: String,
    pub consensus: String,
    pub convergences: Vec<ConsensusPoint>,
    pub divergences: Vec<ConsensusPoint>,
    pub replies: Vec<ProviderReply>,
    pub missing: Vec<String>,
    pub quorum_met: bool,
    pub coverage: usize,
    pub confidence: f64,
    pub elapsed_ms: u64,
    pub started_at: u64,
    pub finished_at: u64,
}

/// Recibo do despacho da ordem física ao `local-daemon` via Named Pipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecutionReceipt {
    pub endpoint: String,
    pub mission_id: Option<String>,
    pub encoding: String,
    pub bytes_written: usize,
    pub dispatched_at: u64,
    pub acknowledged: bool,
    pub response: Option<String>,
}

// --------------------------------------------------------------------------------------
// Motor de orquestração
// --------------------------------------------------------------------------------------

/// Motor blackboard: canal assíncrono de respostas das WebViews + registro de missões.
///
/// Todos os campos são compartilhados via `Arc`, portanto a struct é `Clone` e pode ser
/// movida para tarefas (`tokio::spawn`) sem perder a identidade do estado gerenciado.
#[derive(Clone)]
pub struct MaestroEngine {
    reply_tx: mpsc::Sender<ProviderReply>,
    reply_rx: Arc<Mutex<mpsc::Receiver<ProviderReply>>>,
    missions: Arc<RwLock<HashMap<String, MissionRecord>>>,
    last_outcome: Arc<RwLock<Option<DebateOutcome>>>,
    last_dispatch: Arc<RwLock<Option<PlanExecutionReceipt>>>,
    debate_active: Arc<AtomicBool>,
}

/// Libera a trava de ciclo único mesmo quando a tarefa é cancelada ou entra em panico.
struct DebateGuard<'a>(&'a AtomicBool);

impl Drop for DebateGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Default for MaestroEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MaestroEngine {
    /// Cria o motor e o canal de blackboard usado pelas WebViews.
    pub fn new() -> Self {
        let (reply_tx, reply_rx) = mpsc::channel(REPLY_CHANNEL_CAPACITY);
        Self {
            reply_tx,
            reply_rx: Arc::new(Mutex::new(reply_rx)),
            missions: Arc::new(RwLock::new(HashMap::new())),
            last_outcome: Arc::new(RwLock::new(None)),
            last_dispatch: Arc::new(RwLock::new(None)),
            debate_active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Clonador do transmissor do canal (usado pelas rotas IPC `maestro_receive_reply`).
    pub fn reply_sender(&self) -> mpsc::Sender<ProviderReply> {
        self.reply_tx.clone()
    }

    /// Último consenso produzido, se houver (reidratação da UI após recarregar a página).
    pub async fn last_outcome(&self) -> Option<DebateOutcome> {
        self.last_outcome.read().await.clone()
    }

    /// Último despacho de ordem física, se houver.
    pub async fn last_dispatch(&self) -> Option<PlanExecutionReceipt> {
        self.last_dispatch.read().await.clone()
    }

    /// Registro de uma missão específica.
    pub async fn mission(&self, mission_id: &str) -> Option<MissionRecord> {
        self.missions.read().await.get(mission_id).cloned()
    }

    // ----------------------------------------------------------------------------------
    // Canal de entrada (JS -> Rust)
    // ----------------------------------------------------------------------------------

    /// Recebe o texto lido por uma WebView e o deposita no blackboard.
    ///
    /// `partial = true` indica streaming em andamento: o texto é apenas repassado como
    /// evento de progresso e NÃO conta como resposta consolidada do agente.
    pub async fn submit_reply(
        &self,
        app: &AppHandle,
        provider: &str,
        text: &str,
        mission_id: Option<&str>,
        partial: bool,
    ) -> Result<bool, String> {
        let config = WebviewManager::find_config(provider)
            .ok_or_else(|| format!("Provedor não reconhecido: {provider}"))?;

        let trimmed = text.trim();
        if trimmed.is_empty() {
            warn!("Resposta vazia descartada do provedor [{}]", config.name);
            return Ok(false);
        }

        let cleaned = truncate_chars(trimmed, MAX_REPLY_CHARS);

        if partial {
            let progress = DebateProgress {
                mission_id: mission_id.unwrap_or("stream").to_string(),
                stage: "reply_partial".to_string(),
                provider: Some(config.id.to_string()),
                detail: truncate_chars(&cleaned, MAX_STATEMENT_CHARS),
                received: 0,
                expected: PROVIDERS.len(),
                timestamp: now_ms(),
            };
            let _ = app.emit("maestro-reply-partial", &progress);
            return Ok(true);
        }

        let reply = ProviderReply {
            provider: config.id.to_string(),
            mission_id: mission_id.map(|m| m.to_string()),
            text: cleaned,
            ok: true,
            captured_at: now_ms(),
            latency_ms: 0,
        };

        // `send` só falha se o receptor foi destruído (motor desligado).
        self.reply_tx
            .send(reply)
            .await
            .map_err(|e| format!("Canal de blackboard indisponível: {e}"))?;

        info!(
            "Resposta consolidada recebida de [{}] ({} caracteres) depositada no blackboard.",
            config.name,
            text.trim().chars().count()
        );
        Ok(true)
    }

    // ----------------------------------------------------------------------------------
    // Fase 3 - Ciclo de debate multi-agente
    // ----------------------------------------------------------------------------------

    /// Executa um ciclo completo de debate: injeta o prompt nas WebViews ocultas de todos os provedores,
    /// aguarda as respostas no canal assíncrono e produz o consenso.
    pub async fn run_debate_cycle(
        &self,
        app: &AppHandle,
        prompt: &str,
    ) -> Result<DebateOutcome, String> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err("Prompt vazio: informe a missão a ser debatida.".to_string());
        }

        // Um único ciclo por vez para evitar mistura de respostas no blackboard.
        if self
            .debate_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("Já existe um ciclo de debate em andamento.".to_string());
        }

        // Libera a trava mesmo se a tarefa for cancelada no meio do ciclo.
        let _guard = DebateGuard(&self.debate_active);

        let result = self.debate_cycle_inner(app, prompt).await;

        match &result {
            Ok(outcome) => {
                *self.last_outcome.write().await = Some(outcome.clone());
                emit_log(
                    app,
                    LogLevel::Success,
                    format!(
                        "Consenso [{}] gerado com {} de {} agentes em {} ms.",
                        outcome.mission_id, outcome.coverage, PROVIDERS.len(), outcome.elapsed_ms
                    ),
                );
                let _ = app.emit("maestro-consensus-ready", outcome);
            }
            Err(err) => {
                emit_log(app, LogLevel::Error, format!("Ciclo de debate falhou: {err}"));
                let _ = app.emit("maestro-debate-failed", err.clone());
            }
        }

        result
    }

    async fn debate_cycle_inner(
        &self,
        app: &AppHandle,
        prompt: &str,
    ) -> Result<DebateOutcome, String> {
        let mission_id = new_mission_id();
        let started_at = now_ms();
        let clock = Instant::now();

        emit_log(
            app,
            LogLevel::Info,
            format!("Missão {mission_id} iniciada. Injetando prompt nas sessões ocultas..."),
        );

        // 1. Injeção escalonada com jitter estocástico nas WebViews ocultas.
        let mut injected: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        let schedule = compute_stochastic_jitter_schedule(PROVIDERS.len(), 200, 600);
        let mut prev_delay = Duration::from_millis(0);

        for (idx, config) in PROVIDERS.iter().enumerate() {
            let delay = schedule.get(idx).copied().unwrap_or(Duration::from_millis(0));
            let delta = delay.saturating_sub(prev_delay);
            if !delta.is_zero() {
                tokio::time::sleep(delta).await;
            }
            prev_delay = delay;

            match Self::inject_prompt(app, config, &mission_id, prompt) {
                Ok(()) => {
                    injected.push(config.id.to_string());
                    info!(
                        "Prompt injetado com sucesso na sessão [{}] (jitter acumulado: {}ms)",
                        config.name,
                        delay.as_millis()
                    );
                }
                Err(err) => {
                    warn!("Falha ao injetar prompt em [{}]: {}", config.name, err);
                    missing.push(config.id.to_string());
                }
            }
        }

        if injected.is_empty() {
            return Err(format!(
                "Nenhuma sessão de provedor disponível. Autentique ao menos um agente ({}) antes de iniciar o debate.",
                provider_names_joined("ou")
            ));
        }

        {
            let mut missions = self.missions.write().await;
            // Histórico limitado: descarta a missão mais antiga ao atingir o teto.
            if missions.len() >= MAX_MISSION_HISTORY {
                if let Some(oldest) = missions
                    .values()
                    .min_by_key(|record| record.started_at)
                    .map(|record| record.mission_id.clone())
                {
                    missions.remove(&oldest);
                }
            }
            missions.insert(
                mission_id.clone(),
                MissionRecord {
                    mission_id: mission_id.clone(),
                    prompt: prompt.to_string(),
                    status: "collecting".to_string(),
                    injected: injected.clone(),
                    missing: missing.clone(),
                    received: Vec::new(),
                    started_at,
                    finished_at: None,
                },
            );
        }

        emit_progress(
            app,
            &mission_id,
            "prompt_injected",
            None,
            format!(
                "Prompt injetado em {} de {} sessões.",
                injected.len(),
                PROVIDERS.len()
            ),
            0,
            injected.len(),
        );

        // 2. Coleta das respostas no canal assíncrono (blackboard).
        let injected_set: HashSet<&str> = injected.iter().map(String::as_str).collect();
        let mut collected: HashMap<String, ProviderReply> = HashMap::new();
        let mut late_replies = 0usize;

        {
            let mut rx = self.reply_rx.lock().await;

            // Descarta respostas órfãs de missões anteriores antes de iniciar a coleta.
            while rx.try_recv().is_ok() {}

            let required_quorum = REPLY_QUORUM.min(injected.len());
            let deadline = Instant::now() + DEBATE_TIMEOUT;
            let mut grace_deadline: Option<Instant> = None;

            while collected.len() < injected.len() {
                let effective_deadline = grace_deadline.unwrap_or(deadline);
                let remaining = effective_deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    warn!(
                        "Missão {mission_id}: coleta encerrada com {} de {} respostas.",
                        collected.len(),
                        injected.len()
                    );
                    break;
                }

                match tokio::time::timeout(remaining, rx.recv()).await {
                    Ok(Some(mut reply)) => {
                        if reply.mission_id.as_deref() != Some(mission_id.as_str()) {
                            if reply.mission_id.is_some() {
                                // Resposta de missão anterior: ignora sem interromper o ciclo.
                                late_replies += 1;
                                continue;
                            }
                            // WebView antiga que ainda não recebeu o carimbo da missão:
                            // assume a missão corrente para não perder o resultado.
                            reply.mission_id = Some(mission_id.clone());
                        }

                        if !injected_set.contains(reply.provider.as_str()) {
                            continue;
                        }

                        reply.latency_ms = clock.elapsed().as_millis() as u64;
                        let name = WebviewManager::find_config(&reply.provider)
                            .map(|c| c.name.to_string())
                            .unwrap_or_else(|| reply.provider.clone());

                        info!(
                            "Resposta de [{}] consolidada no blackboard da missão {}.",
                            name, mission_id
                        );

                        let is_new = !collected.contains_key(&reply.provider);
                        collected.insert(reply.provider.clone(), reply);

                        if is_new {
                            emit_progress(
                                app,
                                &mission_id,
                                "reply_received",
                                Some(name),
                                "Resposta capturada pela WebView.".to_string(),
                                collected.len(),
                                injected.len(),
                            );
                        }

                        // Quórum atingido: concede apenas uma janela curta para os demais.
                        if grace_deadline.is_none() && collected.len() >= required_quorum {
                            grace_deadline = Some(Instant::now() + POST_QUORUM_GRACE);
                        }
                    }
                    Ok(None) => {
                        warn!("Canal de blackboard encerrado durante a missão {mission_id}.");
                        break;
                    }
                    Err(_) => break, // timeout global
                }
            }
        }

        if late_replies > 0 {
            emit_log(
                app,
                LogLevel::Warn,
                format!("{late_replies} resposta(s) tardia(s) de missões anteriores foram descartadas."),
            );
        }

        // 3. Consolidação do consenso (Blackboard -> documento único).
        let elapsed_ms = clock.elapsed().as_millis() as u64;
        let mut replies: Vec<ProviderReply> = collected.into_values().collect();
        replies.sort_by(|a, b| a.provider.cmp(&b.provider));

        let received_ids: Vec<String> = replies.iter().map(|r| r.provider.clone()).collect();
        for provider in &injected {
            if !received_ids.contains(provider) {
                if let Some(config) = WebviewManager::find_config(provider) {
                    missing.push(config.id.to_string());
                }
            }
        }
        missing.sort();
        missing.dedup();

        let outcome = build_outcome(
            mission_id.clone(),
            prompt,
            replies,
            missing,
            started_at,
            elapsed_ms,
        );

        {
            let mut missions = self.missions.write().await;
            if let Some(record) = missions.get_mut(&mission_id) {
                record.status = outcome.status.clone();
                record.received = received_ids;
                record.missing = outcome.missing.clone();
                record.finished_at = Some(outcome.finished_at);
            }
        }

        Ok(outcome)
    }

    /// Injeta o prompt em uma sessão web oculta via `eval`, carimbando a missão corrente.
    fn inject_prompt(
        app: &AppHandle,
        config: &'static ProviderConfig,
        mission_id: &str,
        prompt: &str,
    ) -> Result<(), String> {
        let window = WebviewManager::ensure_window(app, config, false)?;

        // A janela deve permanecer oculta e viva durante todo o ciclo.
        let _ = window.hide();

        let prompt_literal =
            serde_json::to_string(prompt).map_err(|e| format!("Prompt inválido: {e}"))?;
        let mission_literal = serde_json::to_string(mission_id)
            .map_err(|e| format!("Identificador de missão inválido: {e}"))?;

        let script = format!(
            r#"(function () {{
  window.postMessage({{
    source: "rdc-maestro",
    action: "send_prompt",
    mission_id: {mission_literal},
    prompt: {prompt_literal}
  }}, "*");
  return true;
}})();"#
        );

        window
            .eval(&script)
            .map_err(|e| format!("eval() rejeitado na sessão [{}]: {e}", config.name))
    }

    // ----------------------------------------------------------------------------------
    // Fase 4 - Despacho da ordem física ao daemon local
    // ----------------------------------------------------------------------------------

    /// Conecta ao Named Pipe nativo do RDC e envia a ordem física aprovada ao daemon.
    pub async fn execute_approved_plan(&self, payload: &str) -> Result<PlanExecutionReceipt, String> {
        let endpoints = daemon_pipe_candidates();
        self.execute_approved_plan_on(payload, &endpoints).await
    }

    /// Variante com endpoints explícitos (usada pelos testes de integração do canal).
    pub async fn execute_approved_plan_on(
        &self,
        payload: &str,
        endpoints: &[String],
    ) -> Result<PlanExecutionReceipt, String> {
        let trimmed = payload.trim();
        if trimmed.is_empty() {
            return Err("Plano aprovado vazio: nada a executar.".to_string());
        }

        let (order, encoding, mission_id) = build_daemon_order(trimmed)?;
        let line = serde_json::to_string(&order)
            .map_err(|e| format!("Falha ao serializar a ordem física: {e}"))?;

        let (endpoint, mut stream) = connect_daemon_pipe(endpoints).await?;

        stream
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| format!("Falha ao escrever a ordem no pipe {endpoint}: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| format!("Falha ao liberar o buffer do pipe {endpoint}: {e}"))?;

        info!(
            "Ordem física ({} bytes, encoding={}) despachada para o daemon via {}.",
            line.len(),
            encoding,
            endpoint
        );

        // O daemon pode (ou não) responder no mesmo pipe; a leitura é oportunista.
        let response = read_daemon_ack(&mut stream).await;

        let receipt = PlanExecutionReceipt {
            endpoint,
            mission_id,
            encoding,
            bytes_written: line.len(),
            dispatched_at: now_ms(),
            acknowledged: response.is_some(),
            response,
        };

        *self.last_dispatch.write().await = Some(receipt.clone());
        Ok(receipt)
    }
}

// --------------------------------------------------------------------------------------
// Consenso determinístico (Blackboard -> documento único)
// --------------------------------------------------------------------------------------

fn build_outcome(
    mission_id: String,
    prompt: &str,
    replies: Vec<ProviderReply>,
    missing: Vec<String>,
    started_at: u64,
    elapsed_ms: u64,
) -> DebateOutcome {
    let coverage = replies.len();
    let quorum_met = coverage >= REPLY_QUORUM;

    let (convergences, divergences) = compute_consensus_points(&replies);

    let status = if coverage == 0 {
        "no_reply"
    } else if quorum_met {
        "consensus"
    } else {
        "partial"
    };

    let denominator = convergences.len() + divergences.len();
    let confidence = if denominator == 0 {
        0.0
    } else {
        convergences.len() as f64 / denominator as f64
    };

    let consensus = render_consensus(
        &mission_id,
        prompt,
        status,
        coverage,
        &missing,
        &convergences,
        &divergences,
        &replies,
    );

    DebateOutcome {
        mission_id,
        prompt: prompt.to_string(),
        status: status.to_string(),
        consensus,
        convergences,
        divergences,
        replies,
        missing,
        quorum_met,
        coverage,
        confidence,
        elapsed_ms,
        started_at,
        finished_at: now_ms(),
    }
}

/// Extrai pontos convergentes (sustentados por 2+ agentes) e divergentes (isolados).
fn compute_consensus_points(replies: &[ProviderReply]) -> (Vec<ConsensusPoint>, Vec<ConsensusPoint>) {
    struct Candidate {
        provider: String,
        text: String,
        terms: BTreeSet<String>,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for reply in replies {
        let mut seen: HashSet<String> = HashSet::new();
        for statement in split_statements(&reply.text) {
            let fingerprint = statement.to_lowercase();
            if !seen.insert(fingerprint) {
                continue;
            }
            let terms = normalize_terms(&statement);
            if terms.len() < 3 {
                continue;
            }
            candidates.push(Candidate {
                provider: reply.provider.clone(),
                text: statement,
                terms,
            });
        }
    }

    let required_support = REPLY_QUORUM.min(replies.len()).max(1);

    let mut convergences: Vec<ConsensusPoint> = Vec::new();
    let mut divergences: Vec<ConsensusPoint> = Vec::new();
    let mut selected: Vec<(BTreeSet<String>, usize)> = Vec::new();

    for candidate in &candidates {
        let mut supporters: Vec<String> = vec![candidate.provider.clone()];
        for other in &candidates {
            if other.provider == candidate.provider {
                continue;
            }
            let similarity = jaccard(&candidate.terms, &other.terms);
            if similarity >= CONVERGENCE_THRESHOLD && !supporters.contains(&other.provider) {
                supporters.push(other.provider.clone());
            }
        }

        supporters.sort();
        let support = supporters.len();

        // Deduplica enunciados equivalentes já registrados.
        let redundant = selected.iter().any(|(terms, _)| {
            let provider_scoped = terms == &candidate.terms;
            provider_scoped || jaccard(terms, &candidate.terms) >= DEDUPE_THRESHOLD
        });

        let point = ConsensusPoint {
            text: candidate.text.clone(),
            providers: supporters,
            support,
        };

        if support >= required_support {
            if redundant {
                continue;
            }
            selected.push((candidate.terms.clone(), support));
            convergences.push(point);
        } else if !redundant {
            selected.push((candidate.terms.clone(), support));
            divergences.push(point);
        }
    }

    convergences.sort_by(|a, b| {
        b.support
            .cmp(&a.support)
            .then_with(|| a.text.len().cmp(&b.text.len()))
    });
    divergences.sort_by(|a, b| a.text.len().cmp(&b.text.len()));

    convergences.truncate(MAX_CONSENSUS_ITEMS);
    divergences.truncate(MAX_CONSENSUS_ITEMS);

    (convergences, divergences)
}

#[allow(clippy::too_many_arguments)]
fn render_consensus(
    mission_id: &str,
    prompt: &str,
    status: &str,
    coverage: usize,
    missing: &[String],
    convergences: &[ConsensusPoint],
    divergences: &[ConsensusPoint],
    replies: &[ProviderReply],
) -> String {
    let mut out = String::new();
    out.push_str("# Consenso Multi-Agente (RDC Maestro Blackboard)\n");
    out.push_str(&format!("Missão: {mission_id}\n"));
    out.push_str(&format!(
        "Status: {} | Cobertura: {}/{} agentes | Quórum mínimo: {}\n",
        match status {
            "consensus" => "CONSENSO CONFIRMADO",
            "partial" => "CONSENSO PARCIAL",
            _ => "SEM RESPOSTAS",
        },
        coverage,
        PROVIDERS.len(),
        REPLY_QUORUM
    ));
    out.push_str(&format!(
        "Prompt: {}\n",
        truncate_chars(prompt.trim(), MAX_STATEMENT_CHARS)
    ));
    if !missing.is_empty() {
        out.push_str(&format!("Agentes ausentes: {}\n", missing.join(", ")));
    }

    out.push_str("\n## Pontos convergentes\n");
    if convergences.is_empty() {
        out.push_str("Nenhum ponto com adesão de múltiplos agentes foi identificado.\n");
    } else {
        for (index, point) in convergences.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}] {}\n",
                index + 1,
                point.providers.join(" + "),
                point.text
            ));
        }
    }

    out.push_str("\n## Pontos divergentes (isolados)\n");
    if divergences.is_empty() {
        out.push_str("Nenhuma divergência relevante entre os agentes.\n");
    } else {
        for point in divergences.iter() {
            out.push_str(&format!("- [{}] {}\n", point.providers.join(" + "), point.text));
        }
    }

    out.push_str("\n## Respostas integrais\n");
    for reply in replies {
        let name = WebviewManager::find_config(&reply.provider)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| reply.provider.clone());
        out.push_str(&format!(
            "\n### {name} ({})\n{}\n",
            reply.provider,
            truncate_chars(&reply.text, MAX_TRANSCRIPT_CHARS)
        ));
    }

    out
}

/// Quebra um texto em enunciados comparáveis (linhas, sentenças e itens de lista).
fn split_statements(text: &str) -> Vec<String> {
    let mut statements: Vec<String> = Vec::new();
    let mut buffer = String::new();

    for ch in text.chars() {
        if matches!(ch, '\n' | '\r' | '.' | '!' | '?' | ';' | '•') {
            push_statement(&mut statements, &buffer);
            buffer.clear();
        } else {
            buffer.push(ch);
        }
    }
    push_statement(&mut statements, &buffer);
    statements
}

fn push_statement(statements: &mut Vec<String>, raw: &str) {
    let cleaned = raw
        .trim()
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '#' | '>' | ')' | '(' | '[' | ']'))
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(|c: char| matches!(c, '.' | ')' | ':' | ' '))
        .trim();

    let words = cleaned.split_whitespace().count();
    if words < 4 {
        return;
    }

    let statement = truncate_chars(cleaned, MAX_STATEMENT_CHARS);
    if !statements.contains(&statement) {
        statements.push(statement);
    }
}

/// Termos normalizados (sem stopwords) usados na similaridade entre enunciados.
fn normalize_terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .map(|token| {
            token
                .trim_matches(|c: char| c == '-' || c == '_')
                .to_lowercase()
        })
        .filter(|token| token.chars().count() >= 3 && !STOPWORDS.contains(&token.as_str()))
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 { 0.0 } else { intersection / union }
}

/// Calcula um cronograma de atrasos estocásticos para escalonar as injeções entre múltiplos provedores,
/// evitando rajadas (bursts) simultâneas no mesmo milissegundo que acionam defesas de correlação de tráfego.
pub fn compute_stochastic_jitter_schedule(
    count: usize,
    min_delay_ms: u64,
    max_delay_ms: u64,
) -> Vec<Duration> {
    if count == 0 {
        return Vec::new();
    }
    let mut schedule = Vec::with_capacity(count);
    schedule.push(Duration::from_millis(0));

    let mut accumulated_ms = 0u64;
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xCAFE_BABE);

    for _ in 1..count {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;

        let span = (max_delay_ms - min_delay_ms).max(1);
        let jitter = min_delay_ms + (seed % span);
        accumulated_ms += jitter;
        schedule.push(Duration::from_millis(accumulated_ms));
    }

    schedule
}

/// Stopwords PT-BR + EN usadas para reduzir ruído na comparação de enunciados.
const STOPWORDS: &[&str] = &[
    // Português
    "aos", "aquela", "aquele", "aquilo", "as", "até", "com", "como", "da", "das", "de", "dela",
    "dele", "depois", "do", "dos", "e", "ela", "elas", "ele", "eles", "em", "entre", "era", "essa",
    "esse", "esta", "está", "este", "eu", "foi", "for", "há", "isso", "isto", "já", "lhe", "mais",
    "mas", "me", "mesmo", "meu", "minha", "muito", "na", "não", "nas", "nem", "no", "nos", "nós",
    "num", "o", "os", "ou", "para", "pela", "pelo", "por", "qual", "quando", "que", "quem", "se",
    "sem", "ser", "seu", "só", "sua", "também", "te", "tem", "ter", "tu", "um", "uma", "você",
    "vocês", "ainda", "assim", "após", "cada", "sobre", "todo", "toda", "todos", "todas", "onde",
    // Inglês
    "and", "the", "for", "are", "but", "not", "you", "your", "with", "this", "that", "from",
    "have", "has", "will", "can", "should", "would", "there", "their", "they", "them", "was",
    "were", "been", "into", "than", "then", "when", "which", "while", "about", "also", "its",
    "it's", "our", "out", "use", "using", "all", "any", "may", "must", "one", "two", "how",
];

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max).collect();
    truncated.push_str("…");
    truncated
}

// --------------------------------------------------------------------------------------
// Fase 4 - Cliente do Named Pipe do daemon
// --------------------------------------------------------------------------------------

/// Monta o envelope JSON-lines da ordem física enviada ao daemon.
///
/// Retorna `(envelope, encoding, mission_id)`. Quando o payload aprovado é JSON válido
/// ele é embutido como objeto em `payload`; caso contrário é enviado como texto puro.
fn build_daemon_order(
    payload: &str,
) -> Result<(serde_json::Value, String, Option<String>), String> {
    let parsed = serde_json::from_str::<serde_json::Value>(payload);
    let (value, encoding) = match parsed {
        Ok(value @ serde_json::Value::Object(_)) => (value, "json"),
        Ok(value) => (value, "json"),
        Err(_) => (serde_json::Value::String(payload.to_string()), "text"),
    };

    let mission_id = serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| {
            value
                .get("mission_id")
                .or_else(|| value.get("missionId"))
                .and_then(|m| m.as_str())
                .map(|m| m.to_string())
        });

    let order = serde_json::json!({
        "schema": DAEMON_ORDER_SCHEMA,
        "source": "rdc-maestro",
        "kind": "execute_approved_plan",
        "issued_at": now_ms(),
        "mission_id": mission_id.clone(),
        "payload_encoding": encoding,
        "payload": value,
    });

    Ok((order, encoding.to_string(), mission_id))
}

/// Lista de endpoints do canal de ordens, permitindo sobrescrever via ambiente
/// (`RDC_MAESTRO_DAEMON_PIPES`, separados por `;`) para depuração com um pipe simulado.
pub fn daemon_pipe_candidates() -> Vec<String> {
    if let Ok(raw) = std::env::var("RDC_MAESTRO_DAEMON_PIPES") {
        let custom: Vec<String> = raw
            .split(';')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect();
        if !custom.is_empty() {
            return custom;
        }
    }
    DAEMON_COMMAND_PIPES.iter().map(|p| p.to_string()).collect()
}

/// Conecta ao primeiro pipe de comando disponível, com retentativas para o caso do
/// daemon ainda estar subindo (ERROR_PIPE_BUSY / ERROR_FILE_NOT_FOUND).
#[cfg(windows)]
async fn connect_daemon_pipe(
    endpoints: &[String],
) -> Result<(String, tokio::net::windows::named_pipe::NamedPipeClient), String> {
    use tokio::net::windows::named_pipe::ClientOptions;

    const ERROR_PIPE_BUSY: i32 = 231;

    if endpoints.is_empty() {
        return Err("Nenhum endpoint de pipe configurado para o canal de ordens.".to_string());
    }

    let mut last_error = String::from("nenhum endpoint foi tentado");

    for attempt in 0..DAEMON_CONNECT_ATTEMPTS {
        for endpoint in endpoints {
            match ClientOptions::new().open(endpoint.as_str()) {
                Ok(client) => {
                    info!("Conectado ao Named Pipe do daemon em {endpoint}.");
                    return Ok((endpoint.clone(), client));
                }
                Err(err) => {
                    let retryable = matches!(err.raw_os_error(), Some(ERROR_PIPE_BUSY) | Some(2));
                    last_error = format!("{endpoint}: {err}");
                    if !retryable {
                        warn!("Pipe {endpoint} indisponível: {err}");
                    }
                }
            }
        }

        if attempt + 1 < DAEMON_CONNECT_ATTEMPTS {
            tokio::time::sleep(DAEMON_CONNECT_BACKOFF).await;
        }
    }

    Err(format!(
        "Nenhum Named Pipe do daemon acessível ({endpoints:?}). Verifique se o `local-daemon` está em execução. Último erro: {last_error}"
    ))
}

#[cfg(not(windows))]
async fn connect_daemon_pipe(endpoints: &[String]) -> Result<(String, tokio::net::UnixStream), String> {
    let _ = endpoints;
    Err(
        "O canal de ordens do daemon usa Named Pipe do Windows; nenhum transporte local equivalente foi configurado nesta plataforma."
            .to_string(),
    )
}

/// Leitura oportunista do reconhecimento do daemon (o pipe pode ser somente-escrita).
///
/// Acumula até encontrar o terminador de linha: o daemon pode entregar o recibo em mais
/// de uma leitura do pipe, e o Maestro precisa do JSON completo para registrar o recibo.
#[cfg(windows)]
async fn read_daemon_ack(
    stream: &mut tokio::net::windows::named_pipe::NamedPipeClient,
) -> Option<String> {
    let mut collected: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 1024];
    let deadline = Instant::now() + DAEMON_ACK_TIMEOUT;

    while collected.len() < MAX_ACK_BYTES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.read(&mut buffer)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(read)) => {
                collected.extend_from_slice(&buffer[..read]);
                if collected.contains(&b'\n') {
                    break;
                }
            }
            Ok(Err(err)) => {
                warn!("Erro ao ler reconhecimento do daemon: {err}");
                break;
            }
            Err(_) => break,
        }
    }

    let text = String::from_utf8_lossy(&collected).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(not(windows))]
async fn read_daemon_ack(stream: &mut tokio::net::UnixStream) -> Option<String> {
    let _ = stream;
    None
}

// --------------------------------------------------------------------------------------
// Utilitários de evento e tempo
// --------------------------------------------------------------------------------------

/// Emite um evento de log consumido pelo console da UI (`maestro-log`).
pub fn emit_log(app: &AppHandle, level: LogLevel, message: impl Into<String>) {
    let payload = MaestroLog {
        level: level.as_str().to_string(),
        message: message.into(),
        timestamp: iso8601_utc(now_ms()),
    };
    if let Err(err) = app.emit("maestro-log", payload) {
        warn!("Falha ao emitir maestro-log: {err}");
    }
}

fn emit_progress(
    app: &AppHandle,
    mission_id: &str,
    stage: &str,
    provider: Option<String>,
    detail: String,
    received: usize,
    expected: usize,
) {
    let payload = DebateProgress {
        mission_id: mission_id.to_string(),
        stage: stage.to_string(),
        provider,
        detail,
        received,
        expected,
        timestamp: now_ms(),
    };
    let _ = app.emit("maestro-debate-progress", payload);
}

/// Identificador de missão monotônico e legível (`m-<epoch_ms>-<contador>`).
pub fn new_mission_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("m-{}-{:04}", now_ms(), seq % 10_000)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}

/// Formata um instante epoch em ISO-8601 UTC sem dependências externas.
pub fn iso8601_utc(epoch_ms: u64) -> String {
    let total_seconds = (epoch_ms / 1000) as i64;
    let days = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (hour, minute, second) = (
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60,
    );

    // Algoritmo civil_from_days (Howard Hinnant) para converter dias epoch em data gregoriana.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// --------------------------------------------------------------------------------------
// Testes unitários do motor (independentes do runtime Tauri)
// --------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(provider: &str, text: &str) -> ProviderReply {
        ProviderReply {
            provider: provider.to_string(),
            mission_id: Some("m-test".to_string()),
            text: text.to_string(),
            ok: true,
            captured_at: 0,
            latency_ms: 10,
        }
    }

    #[test]
    fn splits_and_filters_trivial_statements() {
        let text = "- Reiniciar o serviço de spooler do Windows\nok\n* Validar a conectividade da rede local";
        let statements = split_statements(text);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("Reiniciar o serviço"));
    }

    #[test]
    fn normalizes_terms_without_stopwords() {
        let terms = normalize_terms("Reiniciar o serviço de spooler do Windows e validar a rede");
        assert!(terms.contains("reiniciar"));
        assert!(terms.contains("spooler"));
        assert!(!terms.contains("de"));
        assert!(!terms.contains("a"));
    }

    #[test]
    fn jaccard_is_symmetric_and_bounded() {
        let a = normalize_terms("reiniciar spooler windows servico impressao");
        let b = normalize_terms("reiniciar spooler windows fila impressao");
        let value = jaccard(&a, &b);
        assert!(value > 0.5 && value <= 1.0);
        assert_eq!(value, jaccard(&b, &a));
    }

    #[test]
    fn detects_convergence_between_two_agents() {
        let replies = vec![
            reply(
                "chatgpt",
                "Reiniciar o serviço de spooler do Windows para limpar a fila travada.",
            ),
            reply(
                "claude",
                "Reiniciar o serviço spooler do Windows limpa a fila de impressão travada.",
            ),
            reply("gemini", "Formatar o disco rígido inteiro do servidor."),
        ];

        let (convergences, divergences) = compute_consensus_points(&replies);
        assert!(!convergences.is_empty(), "esperava convergência entre dois agentes");
        assert!(convergences[0].support >= 2);
        assert!(divergences.iter().any(|d| d.providers.contains(&"gemini".to_string())));
    }

    #[test]
    fn outcome_marks_quorum_and_confidence() {
        let replies = vec![
            reply("chatgpt", "Aplicar patch de segurança no servidor de borda imediatamente."),
            reply("claude", "Aplicar patch de segurança no servidor de borda imediatamente agora."),
        ];
        let outcome = build_outcome(
            "m-1".to_string(),
            "Como corrigir a falha?",
            replies,
            vec!["gemini".to_string()],
            0,
            1500,
        );

        assert_eq!(outcome.status, "consensus");
        assert!(outcome.quorum_met);
        assert_eq!(outcome.coverage, 2);
        assert!(outcome.confidence > 0.5);
        assert!(outcome.consensus.contains("CONSENSO CONFIRMADO"));
        assert!(outcome.consensus.contains("Agentes ausentes: gemini"));
    }

    #[test]
    fn outcome_without_replies_is_reported() {
        let outcome = build_outcome("m-2".to_string(), "ping", Vec::new(), Vec::new(), 0, 5);
        assert_eq!(outcome.status, "no_reply");
        assert!(!outcome.quorum_met);
        assert_eq!(outcome.confidence, 0.0);
    }

    #[test]
    fn builds_json_envelope_for_structured_plan() {
        let payload = r#"{"mission_id":"m-42","steps":[{"action":"restart","target":"spooler"}]}"#;
        let (order, encoding, mission_id) = build_daemon_order(payload).unwrap();

        assert_eq!(encoding, "json");
        assert_eq!(mission_id.as_deref(), Some("m-42"));
        assert_eq!(order["schema"], DAEMON_ORDER_SCHEMA);
        assert_eq!(order["source"], "rdc-maestro");
        assert_eq!(order["payload"]["steps"][0]["action"], "restart");
    }

    #[test]
    fn builds_text_envelope_for_free_form_plan() {
        let (order, encoding, mission_id) = build_daemon_order("Reiniciar spooler agora").unwrap();
        assert_eq!(encoding, "text");
        assert!(mission_id.is_none());
        assert_eq!(order["payload"], "Reiniciar spooler agora");
    }

    #[test]
    fn mission_ids_are_unique_and_prefixed() {
        let first = new_mission_id();
        let second = new_mission_id();
        assert!(first.starts_with("m-"));
        assert_ne!(first, second);
    }

    #[test]
    fn formats_iso8601_in_utc() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_700_000_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn truncation_is_char_boundary_safe() {
        let text = "ação".repeat(10);
        let truncated = truncate_chars(&text, 4);
        assert_eq!(truncated.chars().count(), 5); // 4 caracteres + reticências
        assert!(truncated.starts_with("ação"));
    }

    #[test]
    fn default_pipe_candidates_are_used_without_override() {
        let endpoints = daemon_pipe_candidates();
        assert_eq!(endpoints.len(), DAEMON_COMMAND_PIPES.len());
        assert!(endpoints.iter().any(|e| e == r"\\.\pipe\rdc-agent-maestro"));
    }

    /// Fase 4 ponta a ponta: sobe um Named Pipe real, despacha a ordem aprovada pelo
    /// motor e valida o envelope JSON-lines recebido, além do reconhecimento (ack).
    #[cfg(windows)]
    #[tokio::test]
    async fn dispatches_approved_plan_over_named_pipe_and_reads_ack() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::windows::named_pipe::ServerOptions;

        let endpoint = format!(
            r"\\.\pipe\rdc-maestro-test-{}-{}",
            std::process::id(),
            now_ms()
        );

        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&endpoint)
            .expect("falha ao criar o Named Pipe de teste");

        let server_task = tokio::spawn(async move {
            let connected = server;
            connected.connect().await.expect("nenhum cliente conectou ao pipe");

            let mut reader = BufReader::new(connected);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .await
                .expect("falha ao ler a ordem despachada");

            let mut stream = reader.into_inner();
            stream
                .write_all(b"{\"status\":\"accepted\"}\n")
                .await
                .expect("falha ao escrever o ack");
            stream.flush().await.expect("falha ao liberar o ack");
            line
        });

        let engine = MaestroEngine::new();
        let payload = r#"{"mission_id":"m-pipe","steps":[{"action":"restart","target":"spooler"}]}"#;
        let receipt = engine
            .execute_approved_plan_on(payload, std::slice::from_ref(&endpoint))
            .await
            .expect("despacho da ordem falhou");

        let received = tokio::time::timeout(Duration::from_secs(5), server_task)
            .await
            .expect("timeout aguardando o pipe de teste")
            .expect("task do pipe falhou");

        let order: serde_json::Value =
            serde_json::from_str(received.trim()).expect("ordem recebida não é JSON válido");

        assert_eq!(order["schema"], DAEMON_ORDER_SCHEMA);
        assert_eq!(order["source"], "rdc-maestro");
        assert_eq!(order["kind"], "execute_approved_plan");
        assert_eq!(order["mission_id"], "m-pipe");
        assert_eq!(order["payload_encoding"], "json");
        assert_eq!(order["payload"]["steps"][0]["target"], "spooler");
        assert!(order["issued_at"].as_u64().unwrap_or(0) > 0);

        assert_eq!(receipt.endpoint, endpoint);
        assert_eq!(receipt.encoding, "json");
        assert_eq!(receipt.mission_id.as_deref(), Some("m-pipe"));
        assert!(receipt.bytes_written > 0);
        assert!(receipt.acknowledged);
        assert!(receipt.response.unwrap_or_default().contains("accepted"));
        assert!(engine.last_dispatch().await.is_some());
    }

    /// Fase 4: quando nenhum pipe do daemon existe, o despacho falha com diagnóstico claro.
    #[cfg(windows)]
    #[tokio::test]
    async fn dispatch_reports_missing_daemon_pipe() {
        let engine = MaestroEngine::new();
        let missing = vec![format!(
            r"\\.\pipe\rdc-maestro-inexistente-{}-{}",
            std::process::id(),
            now_ms()
        )];

        let error = engine
            .execute_approved_plan_on("Reiniciar spooler", &missing)
            .await
            .expect_err("esperava falha ao conectar em um pipe inexistente");

        assert!(error.contains("Nenhum Named Pipe do daemon acessível"));
        assert!(engine.last_dispatch().await.is_none());
    }
}
