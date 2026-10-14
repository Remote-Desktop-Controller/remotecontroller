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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(windows)]
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{info, warn};

use crate::state_store::{PersistedMission, StateStore};
use crate::webview_manager::{PROVIDERS, ProviderConfig, WebviewManager, provider_names_joined};

// --------------------------------------------------------------------------------------
// Constantes de operação
// --------------------------------------------------------------------------------------

/// Limite máximo de turnos de leitura autônoma (ReAct loop).
pub const MAX_AUTO_READ_TURNS: usize = 3;

/// Capacidade do canal de blackboard (respostas pendentes das WebViews).
pub const REPLY_CHANNEL_CAPACITY: usize = 64;

/// Janela máxima de espera pelas respostas dos agentes antes de consolidar parcialmente (15 minutos).
pub const DEBATE_TIMEOUT: Duration = Duration::from_secs(900);

/// Após o quórum ser atingido, janela extra generosa para aguardar agentes lentos ou pensando (Extended Thinking).
pub const POST_QUORUM_GRACE: Duration = Duration::from_secs(300);

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
pub const DAEMON_COMMAND_PIPES: &[&str] =
    &[r"\\.\pipe\rdc-agent-maestro", r"\\.\pipe\rdc-telemetry"];

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
    #[serde(default = "default_turn_count")]
    pub turns_count: usize,
    #[serde(default)]
    pub context_log: Option<serde_json::Value>,
}

fn default_turn_count() -> usize {
    1
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
    state_store: Arc<StateStore>,
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
            state_store: Arc::new(StateStore::new()),
        }
    }

    /// Cria uma instância com StateStore customizado (testes).
    pub fn with_state_store(state_store: StateStore) -> Self {
        let (reply_tx, reply_rx) = mpsc::channel(REPLY_CHANNEL_CAPACITY);
        Self {
            reply_tx,
            reply_rx: Arc::new(Mutex::new(reply_rx)),
            missions: Arc::new(RwLock::new(HashMap::new())),
            last_outcome: Arc::new(RwLock::new(None)),
            last_dispatch: Arc::new(RwLock::new(None)),
            debate_active: Arc::new(AtomicBool::new(false)),
            state_store: Arc::new(state_store),
        }
    }

    /// Retorna referência ao repositório durável de estado.
    pub fn state_store(&self) -> &StateStore {
        &self.state_store
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

    /// Executa o ciclo de debate multi-agente com suporte a loop autônomo ReAct (até 3 turnos),
    /// preflight de workspace index e persistência de histórico durável.
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

        let mission_id = new_mission_id();
        let mut base_prompt = prompt.to_string();

        // 0a. Pré-leitura de arquivos explicitamente mencionados no prompt (ex: caminhos absolutos ou relativos)
        if let Some(mentioned_files) = preflight_read_mentioned_files(prompt) {
            emit_log(
                app,
                LogLevel::Success,
                "Arquivo(s) mencionado(s) no prompt pré-carregado(s) diretamente do disco para as IAs.",
            );
            base_prompt.push_str(&mentioned_files);
        }

        // 0b. Preflight do workspace index (timeout 1500ms, fallback silencioso)
        let preflight_info = preflight_workspace_index().await;
        if let Some(ref index) = preflight_info {
            emit_log(
                app,
                LogLevel::Info,
                "Workspace index pré-carregado via daemon local.",
            );
            base_prompt.push_str("\n\n[ARQUIVOS DO WORKSPACE NO PC]:\n");
            base_prompt.push_str(index);
        }

        // 0c. Incorpora instruções de protocolo para que as IAs saibam operar ferramentas locais
        base_prompt.push_str("\n\n");
        base_prompt.push_str(PROTOCOL_INSTRUCTIONS);

        // 0d. Incorpora cabeçalho compactado de sessões anteriores se disponível
        if let Some(compacted) = self.state_store.get_compacted_header().await {
            base_prompt = format!("[CONTEXTO DE SESSÕES ANTERIORES]:\n{compacted}\n\n{base_prompt}");
        }

        let mut current_turn = 1;
        let mut current_prompt = format!(
            "{base_prompt}\n\n[RODADA 1 DO DEBATE - ANÁLISE, PROPOSTA E AUTO-ATRIBUIÇÃO DE FUNÇÕES]:\n1. Analisem criticamente o pedido e o código fornecido.\n2. Identifiquem as falhas de mecânica, bugs, gargalos de performance e oportunidades de melhoria.\n3. Cada IA DEVE escolher e declarar explicitamente uma função especializada no time (ex: Mecânicas & Combate, Visual & Partículas/Canvas, Performance/Game Loop, ou Arquitetura/HUD).\n4. NÃO entreguem o código final completo ainda nesta rodada. Apresentem a estratégia técnica e o que farão na sua parte."
        );
        let mut final_outcome: Option<DebateOutcome> = None;

        while current_turn <= MAX_AUTO_READ_TURNS {
            emit_log(
                app,
                LogLevel::Info,
                format!("Iniciando Turno {current_turn} da missão {mission_id}..."),
            );

            let outcome = match self.debate_cycle_inner(app, &mission_id, &current_prompt).await {
                Ok(mut o) => {
                    o.turns_count = current_turn;
                    o
                }
                Err(err) => {
                    emit_log(
                        app,
                        LogLevel::Error,
                        format!("Ciclo de debate falhou no turno {current_turn}: {err}"),
                    );
                    let _ = app.emit("maestro-debate-failed", err.clone());
                    return Err(err);
                }
            };

            let (has_readonly, has_mutating, readonly_steps) = classify_tools(&outcome.consensus);

            // ReAct Loop: se alguma IA solicitou leitura de arquivos via MCP local
            if has_readonly && !readonly_steps.is_empty() && current_turn < MAX_AUTO_READ_TURNS {
                emit_log(
                    app,
                    LogLevel::Info,
                    format!(
                        "Turno {current_turn}: Consenso gerou {} leitura(s) autônoma(s). Consultando daemon local...",
                        readonly_steps.len()
                    ),
                );

                let auto_read_payload = serde_json::json!({
                    "mission_id": mission_id,
                    "turn": current_turn,
                    "steps": readonly_steps,
                });
                let _ = app.emit("maestro-auto-read-started", &auto_read_payload);

                let read_order = serde_json::json!({
                    "schema": DAEMON_ORDER_SCHEMA,
                    "source": "rdc-maestro",
                    "kind": "auto_read",
                    "mission_id": mission_id,
                    "payload": {
                        "steps": readonly_steps
                    }
                });

                let read_str = serde_json::to_string(&read_order).unwrap_or_default();
                if let Ok(receipt) = self.execute_approved_plan(&read_str).await {
                    if receipt.acknowledged {
                        let parsed_ack: serde_json::Value = receipt
                            .response
                            .as_deref()
                            .and_then(|r| serde_json::from_str(r).ok())
                            .unwrap_or_default();

                        let observations = format_read_observations(&parsed_ack);
                        emit_log(
                            app,
                            LogLevel::Success,
                            format!(
                                "Turno {current_turn}: Observações do PC coletadas. Realimentando agentes para a próxima rodada..."
                            ),
                        );

                        let mut replies_summary = String::new();
                        for r in &outcome.replies {
                            let name = WebviewManager::find_config(&r.provider)
                                .map(|c| c.name.to_string())
                                .unwrap_or_else(|| r.provider.clone());
                            replies_summary.push_str(&format!("### Posição de [{name}]:\n{}\n\n", truncate_chars(&r.text, 2500)));
                        }

                        current_prompt = format!(
                            "{base_prompt}\n\n[OBSERVAÇÕES AUTÔNOMAS DO PC - TURNO {current_turn}]:\n{observations}\n\n[RESUMO DO CONSELHO NA RODADA ANTERIOR]:\n{replies_summary}\nContinuem o debate e aprimorem suas propostas com base nessas leituras reais do computador."
                        );
                        final_outcome = Some(outcome);
                        current_turn += 1;
                        continue;
                    }
                }
            }

            // Debate Multi-Rodadas: avança pelos estágios colaborativos até a síntese final
            if current_turn == 1 {
                emit_log(
                    app,
                    LogLevel::Info,
                    "Rodada 1 concluída: IAs apresentaram suas análises e papéis. Iniciando Rodada 2 de debate cruzado e implementação...".to_string(),
                );

                let mut replies_summary = String::new();
                for r in &outcome.replies {
                    let name = WebviewManager::find_config(&r.provider)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| r.provider.clone());
                    replies_summary.push_str(&format!("### Análise e Papel Assumido por [{name}]:\n{}\n\n", truncate_chars(&r.text, 2500)));
                }

                current_prompt = format!(
                    "{base_prompt}\n\n[RODADA 2 DO DEBATE - CONFRONTO DE IDEIAS E IMPLEMENTAÇÃO]:\nO conselho de IAs analisou a missão e propôs as seguintes abordagens na Rodada 1:\n\n{replies_summary}\nAgora, conversem e debatam diretamente entre si:\n1. Critiquem os pontos fracos dos colegas e concordem na arquitetura unificada para que tudo se encaixe perfeitamente.\n2. Implemente detalhadamente o código da função que você assumiu no time, garantindo que suas variáveis e estruturas funcionem com as dos outros membros.\n3. Apresente o código da sua parte especializada."
                );
                final_outcome = Some(outcome);
                current_turn = 2;
                continue;
            } else if current_turn == 2 && !has_mutating {
                emit_log(
                    app,
                    LogLevel::Info,
                    "Rodada 2 concluída: IAs debateram e implementaram suas partes. Iniciando Rodada 3 para síntese final e montagem unificada...".to_string(),
                );

                let mut replies_summary = String::new();
                for r in &outcome.replies {
                    let name = WebviewManager::find_config(&r.provider)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| r.provider.clone());
                    replies_summary.push_str(&format!("### Implementação de [{name}]:\n{}\n\n", truncate_chars(&r.text, 3000)));
                }

                current_prompt = format!(
                    "{base_prompt}\n\n[RODADA 3 DO DEBATE - SÍNTESE FINAL, HARMONIZAÇÃO E ENCAIXE UNIFICADO]:\nAqui estão as implementações e decisões consolidadas na Rodada 2:\n\n{replies_summary}\nCom base nas contribuições do time, integrem todas as partes em uma solução final única, coesa e completa.\nEntreguem agora o context_log e o código executável final (bloco cirúrgico <<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE ou bloco completo {{\"tool\": \"filesystem.write\", ...}})."
                );
                final_outcome = Some(outcome);
                current_turn = 3;
                continue;
            } else {
                final_outcome = Some(outcome);
                break;
            }
        }

        let mut outcome = final_outcome.ok_or_else(|| "Nenhum resultado de debate foi produzido.".to_string())?;
        outcome.turns_count = current_turn;

        *self.last_outcome.write().await = Some(outcome.clone());

        // Grava no armazenamento durável de estado
        let persisted = PersistedMission {
            mission_id: outcome.mission_id.clone(),
            prompt: prompt.to_string(),
            status: outcome.status.clone(),
            turns_count: outcome.turns_count,
            started_at: outcome.started_at,
            finished_at: Some(outcome.finished_at),
            consensus: Some(outcome.consensus.clone()),
            last_checkpoint_id: None,
            context_log: outcome.context_log.clone(),
            execution_ack: None,
        };
        let _ = self.state_store.record_mission(persisted).await;

        emit_log(
            app,
            LogLevel::Success,
            format!(
                "Consenso [{}] pronto após {} turno(s) com {} de {} agentes em {} ms.",
                outcome.mission_id,
                outcome.turns_count,
                outcome.coverage,
                PROVIDERS.len(),
                outcome.elapsed_ms
            ),
        );
        let _ = app.emit("maestro-consensus-ready", &outcome);

        Ok(outcome)
    }

    async fn debate_cycle_inner(
        &self,
        app: &AppHandle,
        mission_id: &str,
        prompt: &str,
    ) -> Result<DebateOutcome, String> {
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
            let delay = schedule
                .get(idx)
                .copied()
                .unwrap_or(Duration::from_millis(0));
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
            if let Some(record) = missions.get_mut(mission_id) {
                record.prompt = prompt.to_string();
                record.status = "collecting".to_string();
            } else {
                #[allow(clippy::collapsible_if)]
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
                    mission_id.to_string(),
                    MissionRecord {
                        mission_id: mission_id.to_string(),
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
                        if reply.mission_id.as_deref() != Some(mission_id) {
                            if reply.mission_id.is_some() {
                                // Resposta de missão anterior: ignora sem interromper o ciclo.
                                late_replies += 1;
                                continue;
                            }
                            // WebView antiga que ainda não recebeu o carimbo da missão:
                            // assume a missão corrente para não perder o resultado.
                            reply.mission_id = Some(mission_id.to_string());
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
                format!(
                    "{late_replies} resposta(s) tardia(s) de missões anteriores foram descartadas."
                ),
            );
        }

        // 3. Consolidação do consenso (Blackboard -> documento único).
        let elapsed_ms = clock.elapsed().as_millis() as u64;
        let mut replies: Vec<ProviderReply> = collected.into_values().collect();
        replies.sort_by(|a, b| a.provider.cmp(&b.provider));

        let received_ids: Vec<String> = replies.iter().map(|r| r.provider.clone()).collect();
        for provider in &injected {
            #[allow(clippy::collapsible_if)]
            if !received_ids.contains(provider) {
                if let Some(config) = WebviewManager::find_config(provider) {
                    missing.push(config.id.to_string());
                }
            }
        }
        missing.sort();
        missing.dedup();

        let outcome = build_outcome(
            mission_id.to_string(),
            prompt,
            replies,
            missing,
            started_at,
            elapsed_ms,
        );

        {
            let mut missions = self.missions.write().await;
            if let Some(record) = missions.get_mut(mission_id) {
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
    pub async fn execute_approved_plan(
        &self,
        payload: &str,
    ) -> Result<PlanExecutionReceipt, String> {
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

        if let Some(ref mid) = receipt.mission_id {
            let parsed_ack: Option<serde_json::Value> = receipt
                .response
                .as_deref()
                .and_then(|r| serde_json::from_str(r).ok());
            let checkpoint_id = parsed_ack
                .as_ref()
                .and_then(|p| p.get("checkpoint_id"))
                .and_then(|c| c.as_str())
                .map(String::from);
            let status = if receipt.acknowledged {
                if parsed_ack.as_ref().and_then(|p| p.get("status")).and_then(|s| s.as_str()) == Some("failed") {
                    "failed"
                } else {
                    "executed"
                }
            } else {
                "dispatched"
            };
            let _ = self.state_store.update_execution_result(mid, status, parsed_ack, checkpoint_id).await;
        }

        *self.last_dispatch.write().await = Some(receipt.clone());
        Ok(receipt)
    }

    /// Executa o rollback de um checkpoint_id contra o daemon e atualiza o histórico durável.
    pub async fn rollback_checkpoint(&self, checkpoint_id: &str) -> Result<PlanExecutionReceipt, String> {
        let order = serde_json::json!({
            "schema": DAEMON_ORDER_SCHEMA,
            "source": "rdc-maestro",
            "kind": "rollback_checkpoint",
            "checkpoint_id": checkpoint_id,
            "payload": {}
        });
        let payload_str = serde_json::to_string(&order).map_err(|e| e.to_string())?;
        let receipt = self.execute_approved_plan(&payload_str).await?;

        // Atualiza a missão correspondente para "rolled_back"
        let history = self.state_store.get_history().await;
        for m in history {
            if m.last_checkpoint_id.as_deref() == Some(checkpoint_id) {
                let _ = self.state_store.update_execution_result(&m.mission_id, "rolled_back", None, None).await;
                break;
            }
        }

        Ok(receipt)
    }

    /// Compacta o histórico em um resumo executivo e reinicia as sessões dos provedores (novo chat).
    pub async fn compact_and_reset_sessions(
        &self,
        app: &AppHandle,
        summary_override: Option<String>,
    ) -> Result<String, String> {
        let history = self.state_store.get_history().await;
        let count = history.len();
        let header = if let Some(override_text) = summary_override {
            override_text
        } else {
            let last_summary = history
                .first()
                .map(|m| {
                    format!(
                        "Missão {}: {}\nStatus: {}\nConsenso: {}",
                        m.mission_id,
                        truncate_chars(&m.prompt, 150),
                        m.status,
                        truncate_chars(m.consensus.as_deref().unwrap_or(""), 400)
                    )
                })
                .unwrap_or_else(|| "Nenhuma missão anterior gravada.".to_string());
            format!("Resumo de sessões anteriores ({count} missões):\n{last_summary}")
        };
        self.state_store.set_compacted_header(Some(header.clone())).await?;

        // Reseta WebViews dos provedores
        for config in PROVIDERS {
            if let Some(window) = app.get_webview_window(config.label) {
                let script = r#"(function() {
                    try {
                        if (window.__MAESTRO_BRIDGE__ && typeof window.__MAESTRO_BRIDGE__.startFreshChat === "function") {
                            window.__MAESTRO_BRIDGE__.startFreshChat();
                        } else {
                            window.postMessage({ source: "rdc-maestro", action: "start_fresh_chat" }, "*");
                        }
                    } catch(e) {
                        console.error("Falha ao resetar chat:", e);
                    }
                })();"#;
                let _ = window.eval(script);
            }
        }

        *self.last_outcome.write().await = None;

        emit_log(
            app,
            LogLevel::Success,
            format!("Histórico compactado ({count} missões) e sessões reiniciadas com sucesso."),
        );

        Ok(header)
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
        consensus: consensus.clone(),
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
        turns_count: 1,
        context_log: extract_context_log_from_text(&consensus, prompt),
    }
}

/// Extrai pontos convergentes (sustentados por 2+ agentes) e divergentes (isolados).
fn compute_consensus_points(
    replies: &[ProviderReply],
) -> (Vec<ConsensusPoint>, Vec<ConsensusPoint>) {
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
    divergences.sort_by_key(|a| a.text.len());

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
            out.push_str(&format!(
                "- [{}] {}\n",
                point.providers.join(" + "),
                point.text
            ));
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
        .trim_start_matches(['-', '*', '#', '>', ')', '(', '[', ']'])
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', ')', ':', ' '])
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
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
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
    "and", "the", "for", "are", "but", "not", "you", "your", "with", "this", "that", "from", "have",
    "has", "will", "can", "should", "would", "there", "their", "they", "them", "was", "were",
    "been", "into", "than", "then", "when", "which", "while", "about", "also", "its", "it's",
    "our", "out", "use", "using", "all", "any", "may", "must", "one", "two", "how",
];

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max).collect();
    truncated.push('…');
    truncated
}

// --------------------------------------------------------------------------------------
// ReAct Loop, Pré-voo de Workspace e Classificação de Ferramentas
// --------------------------------------------------------------------------------------

/// Instruções do protocolo de 2 fases do RDC Maestro injetadas no prompt para orientar os LLMs.
pub const PROTOCOL_INSTRUCTIONS: &str = r#"
[PROTOCOLO DE DOIS ESTÁGIOS - RDC MAESTRO MCP]:
Você é um agente com acesso ao computador local através do RDC Maestro.
- FASE 1 (Inspeção): Se precisar inspecionar ou ler arquivos adicionais no PC antes de formular o código, responda APENAS com o bloco JSON:
```json
[
  {"tool": "filesystem.read", "input": {"path": "caminho/do/arquivo"}}
]
```
O Maestro buscará o arquivo no computador em milissegundos e devolverá o conteúdo no próximo turno autônomo.
- FASE 2 (Execução): Quando tiver as informações necessárias para resolver a tarefa, apresente:
1. O registro de intenção:
```json
"context_log": {
  "intent": "explicação da alteração",
  "files_inspected": ["caminho"],
  "verification_plan": "como verificar"
}
```
2. O patch cirúrgico no formato SEARCH/REPLACE:
<<<<<<< SEARCH: caminho/do/arquivo
trecho original existente
=======
trecho novo substituído
>>>>>>> REPLACE
ou o bloco JSON {"tool": "filesystem.write", "input": {"path": "...", "content": "..."}} caso seja um arquivo novo.
"#;

/// Extrai e lê arquivos mencionados explicitamente no prompt (ex: "C:\caminho\arquivo.ext" ou caminhos relativos).
/// Lê até 150 KB por arquivo com segurança para alimentar o primeiro turno do debate.
pub fn preflight_read_mentioned_files(prompt: &str) -> Option<String> {
    let mut result = String::new();
    let mut found_paths = Vec::new();

    // 1. Caminhos entre aspas: "C:\..." ou "arquivo.ext" ou "caminho/..."
    for cap in prompt.split('"').enumerate() {
        if cap.0 % 2 == 1 {
            let candidate = cap.1.trim();
            if candidate.contains('.') && !candidate.contains('\n') && candidate.len() < 260 {
                found_paths.push(candidate.to_string());
            }
        }
    }

    // 2. Caminhos absolutos do Windows sem aspas: C:\...
    for word in prompt.split_whitespace() {
        let trimmed = word.trim_matches(|c| c == '\'' || c == '"' || c == '(' || c == ')' || c == '[' || c == ']' || c == '<' || c == '>');
        if (trimmed.len() > 3 && trimmed.chars().nth(1) == Some(':') && (trimmed.chars().nth(2) == Some('\\') || trimmed.chars().nth(2) == Some('/')))
            || (trimmed.contains('.') && (trimmed.contains('\\') || trimmed.contains('/')))
        {
            if !found_paths.iter().any(|p| p == trimmed) {
                found_paths.push(trimmed.to_string());
            }
        }
    }

    for path_str in found_paths {
        let mut target_path = std::path::PathBuf::from(&path_str);
        if !target_path.is_file() {
            // Tenta caminhos relativos ao diretório de trabalho ou seus ancestrais
            if let Ok(cur) = std::env::current_dir() {
                let c1 = cur.join(&path_str);
                let c2 = cur.join("../../").join(&path_str);
                let c3 = cur.join("..").join(&path_str);
                if c1.is_file() {
                    target_path = c1;
                } else if c2.is_file() {
                    target_path = c2;
                } else if c3.is_file() {
                    target_path = c3;
                }
            }
        }

        if target_path.is_file() {
            if let Ok(metadata) = std::fs::metadata(&target_path) {
                if metadata.len() <= 250_000 {
                    if let Ok(content) = std::fs::read_to_string(&target_path) {
                        result.push_str(&format!(
                            "\n\n[CONTEÚDO DO ARQUIVO LOCAL ({path_str})]:\n```\n{content}\n```\n"
                        ));
                    }
                } else if let Ok(mut file) = std::fs::File::open(&target_path) {
                    use std::io::Read;
                    let mut buf = vec![0u8; 150_000];
                    if let Ok(n) = file.read(&mut buf) {
                        let content = String::from_utf8_lossy(&buf[..n]);
                        result.push_str(&format!(
                            "\n\n[CONTEÚDO DO ARQUIVO LOCAL ({path_str}) - PRIMEIROS 150KB]:\n```\n{content}\n```\n"
                        ));
                    }
                }
            }
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Pré-voo do índice de arquivos do workspace via Named Pipe (`filesystem.list`, timeout 1500ms).
/// Executado de forma não bloqueante antes do primeiro turno com fallback silencioso.
pub async fn preflight_workspace_index() -> Option<String> {
    let endpoints = daemon_pipe_candidates();
    let order = serde_json::json!({
        "schema": DAEMON_ORDER_SCHEMA,
        "source": "rdc-maestro",
        "kind": "inspect_workspace",
        "payload": {
            "steps": [{
                "tool": "filesystem.list",
                "input": { "path": "." }
            }]
        }
    });

    let line = serde_json::to_string(&order).ok()?;

    let query_fut = async {
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;
            for ep in &endpoints {
                if let Ok(mut client) = ClientOptions::new().open(ep.as_str()) {
                    let write_res = client.write_all(format!("{line}\n").as_bytes()).await;
                    if write_res.is_ok() && client.flush().await.is_ok() {
                        if let Some(ack) = read_daemon_ack(&mut client).await {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&ack) {
                                if parsed["status"] == "completed" {
                                    if let Some(preview) = parsed["steps"][0]["output_preview"].as_str() {
                                        return Some(preview.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            None
        }
        #[cfg(not(windows))]
        {
            None
        }
    };

    match tokio::time::timeout(Duration::from_millis(1500), query_fut).await {
        Ok(res) => res,
        Err(_) => None,
    }
}

/// Formata as observações capturadas pelo daemon em uma string legível para realimentar o turno seguinte.
pub fn format_read_observations(ack_json: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(steps) = ack_json.get("steps").and_then(|s| s.as_array()) {
        for (i, step) in steps.iter().enumerate() {
            let tool = step.get("tool").and_then(|t| t.as_str()).unwrap_or("tool");
            let status = step.get("status").and_then(|s| s.as_str()).unwrap_or("ok");
            let preview = step
                .get("output_preview")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            out.push_str(&format!(
                "Passo {}: [{}] status={}\n{}\n\n",
                i + 1,
                tool,
                status,
                preview
            ));
        }
    } else if let Some(preview) = ack_json.get("output_preview").and_then(|p| p.as_str()) {
        out.push_str(preview);
    } else {
        out.push_str(&ack_json.to_string());
    }
    out
}

/// Classifica os passos propostos no texto de consenso em ferramentas de somente-leitura ou mutações físicas.
pub fn classify_tools(text: &str) -> (bool, bool, Vec<serde_json::Value>) {
    let mut has_readonly = false;
    let mut has_mutating = false;
    let mut readonly_steps: Vec<serde_json::Value> = Vec::new();

    // 1. SEARCH/REPLACE é sempre uma mutação (filesystem.patch)
    if text.contains("<<<<<<< SEARCH") {
        has_mutating = true;
    }

    // 2. Extrai passos estruturados de blocos JSON
    let candidate_steps = extract_candidate_steps(text);
    for step in candidate_steps {
        if let Some(tool) = step.get("tool").and_then(|t| t.as_str()) {
            let tool_lower = tool.to_lowercase();
            if is_readonly_tool_name(&tool_lower) {
                has_readonly = true;
                readonly_steps.push(step);
            } else if is_mutating_tool_name(&tool_lower) {
                has_mutating = true;
            }
        }
    }

    // 3. Fallback textual quando os agentes não formatam em JSON válido
    let lower = text.to_lowercase();
    if lower.contains("filesystem.write")
        || lower.contains("filesystem.patch")
        || lower.contains("filesystem.batch_edit")
        || lower.contains("filesystem.delete")
        || lower.contains("process.spawn")
        || lower.contains("process.exec")
        || lower.contains("checkpoint.rollback")
        || lower.contains("checkpoint.restore")
    {
        has_mutating = true;
    }
    if lower.contains("filesystem.read")
        || lower.contains("filesystem.read_range")
        || lower.contains("filesystem.list")
        || lower.contains("filesystem.search")
        || lower.contains("filesystem.stat")
        || lower.contains("git.status")
        || lower.contains("git.diff")
        || lower.contains("git.show")
        || lower.contains("context.overview")
        || lower.contains("code.inspect")
        || lower.contains("code.symbols")
        || lower.contains("git.log")
    {
        has_readonly = true;
    }

    (has_readonly, has_mutating, readonly_steps)
}

fn is_readonly_tool_name(tool: &str) -> bool {
    let lower = tool.to_lowercase();
    matches!(
        lower.as_str(),
        "filesystem.read"
            | "filesystem.read_range"
            | "filesystem.list"
            | "filesystem.search"
            | "filesystem.stat"
            | "git.status"
            | "git.diff"
            | "git.show"
            | "git.log"
            | "context.overview"
            | "code.inspect"
            | "code.symbols"
            | "read"
            | "list"
            | "search"
            | "stat"
            | "dir"
            | "cat"
    )
}

fn is_mutating_tool_name(tool: &str) -> bool {
    let lower = tool.to_lowercase();
    matches!(
        lower.as_str(),
        "filesystem.write"
            | "filesystem.patch"
            | "filesystem.batch_edit"
            | "filesystem.delete"
            | "process.spawn"
            | "process.exec"
            | "checkpoint.rollback"
            | "checkpoint.restore"
            | "write"
            | "patch"
            | "batch_edit"
            | "delete"
            | "spawn"
            | "exec"
            | "run"
            | "bash"
            | "cmd"
            | "powershell"
            | "rollback"
    )
}

fn extract_candidate_steps(text: &str) -> Vec<serde_json::Value> {
    let mut steps = Vec::new();

    let mut cursor = 0;
    while let Some(start) = text[cursor..].find("```") {
        let abs_start = cursor + start;
        let fence_open = abs_start + 3;
        let content_start = if let Some(nl) = text[fence_open..].find('\n') {
            fence_open + nl + 1
        } else {
            fence_open
        };
        if let Some(end) = text[content_start..].find("```") {
            let abs_end = content_start + end;
            let block = text[content_start..abs_end].trim();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(block) {
                collect_steps_from_json(&val, &mut steps);
            }
            cursor = abs_end + 3;
        } else {
            break;
        }
    }

    if steps.is_empty() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.trim()) {
            collect_steps_from_json(&val, &mut steps);
        }
    }

    steps
}

fn collect_steps_from_json(val: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
    if let Some(arr) = val.get("steps").and_then(|s| s.as_array()) {
        for item in arr {
            if item.is_object() {
                out.push(item.clone());
            }
        }
    } else if let Some(payload_steps) = val.get("payload").and_then(|p| p.get("steps")).and_then(|s| s.as_array()) {
        for item in payload_steps {
            if item.is_object() {
                out.push(item.clone());
            }
        }
    } else if let Some(arr) = val.as_array() {
        for item in arr {
            if item.is_object() && item.get("tool").is_some() {
                out.push(item.clone());
            }
        }
    } else if val.get("tool").is_some() {
        out.push(val.clone());
    }
}

/// Extrai ou infere o objeto `context_log` do plano (garantindo conformidade com a gate do daemon).
pub fn extract_context_log_from_text(text: &str, prompt: &str) -> Option<serde_json::Value> {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        if let Some(ctx) = val.get("context_log") {
            return Some(ctx.clone());
        }
    }

    if let Some(pos) = text.find("\"context_log\"") {
        if let Some(start) = text[..pos].rfind('{') {
            if let Some(end) = text[pos..].find('}') {
                let chunk = &text[start..=pos + end];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(chunk) {
                    if let Some(ctx) = val.get("context_log") {
                        return Some(ctx.clone());
                    }
                }
            }
        }
    }

    let mut files: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<<<<<<< SEARCH") {
            let path = trimmed
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
            if !path.is_empty() && !files.contains(&path) {
                files.push(path);
            }
        }
    }

    // Se não encontrou em SEARCH/REPLACE, procura em passos JSON estruturados
    if files.is_empty() {
        let candidate_steps = extract_candidate_steps(text);
        for step in candidate_steps {
            let input = step.get("input").or_else(|| step.get("args"));
            if let Some(p) = input.and_then(|i| i.get("path")).and_then(|v| v.as_str()) {
                let path_str = p.trim().trim_matches('`').trim_matches('"').trim_matches('\'');
                if !path_str.is_empty() && !files.iter().any(|f| f == path_str) {
                    files.push(path_str.to_string());
                }
            }
        }
    }

    if !files.is_empty() {
        return Some(serde_json::json!({
            "intent": truncate_chars(prompt.trim(), 200),
            "files_inspected": files,
            "verification_plan": "Validar sintaxe e integridade após SEARCH/REPLACE"
        }));
    }

    None
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

    // Se já é um envelope de ordem completo, preserva e garante context_log se necessário
    if let serde_json::Value::Object(ref map) = value {
        if map.get("schema").and_then(|s| s.as_str()) == Some(DAEMON_ORDER_SCHEMA) {
            let mission_id = map.get("mission_id").and_then(|m| m.as_str()).map(String::from);
            let mut full_order = value.clone();
            if full_order.get("context_log").is_none() {
                if let Some(ctx) = extract_context_log_from_text(payload, "Maestro Execution") {
                    full_order["context_log"] = ctx;
                }
            }
            return Ok((full_order, encoding.to_string(), mission_id));
        }
    }

    let mission_id = match &value {
        serde_json::Value::Object(map) => map
            .get("mission_id")
            .or_else(|| map.get("missionId"))
            .and_then(|m| m.as_str())
            .map(String::from),
        _ => None,
    };

    let kind = match &value {
        serde_json::Value::Object(map) => map
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("execute_approved_plan"),
        _ => "execute_approved_plan",
    };

    let checkpoint_id = match &value {
        serde_json::Value::Object(map) => map.get("checkpoint_id").and_then(|c| c.as_str()).map(String::from),
        _ => None,
    };

    let mut context_log = match &value {
        serde_json::Value::Object(map) => map.get("context_log").cloned(),
        _ => None,
    };

    if context_log.is_none() {
        context_log = extract_context_log_from_text(payload, "Maestro execution");
    }

    let inner_payload = match &value {
        serde_json::Value::Object(map) if map.contains_key("payload") => {
            map.get("payload").cloned().unwrap()
        }
        _ => value.clone(),
    };

    let mut order = serde_json::json!({
        "schema": DAEMON_ORDER_SCHEMA,
        "source": "rdc-maestro",
        "kind": kind,
        "issued_at": now_ms(),
        "mission_id": mission_id.clone(),
        "payload_encoding": encoding,
        "payload": inner_payload,
    });

    if let Some(cp) = checkpoint_id {
        order["checkpoint_id"] = serde_json::Value::String(cp);
    }
    if let Some(ctx) = context_log {
        order["context_log"] = ctx;
    }

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
async fn connect_daemon_pipe(
    endpoints: &[String],
) -> Result<(String, tokio::net::UnixStream), String> {
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
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
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
        assert!(
            !convergences.is_empty(),
            "esperava convergência entre dois agentes"
        );
        assert!(convergences[0].support >= 2);
        assert!(
            divergences
                .iter()
                .any(|d| d.providers.contains(&"gemini".to_string()))
        );
    }

    #[test]
    fn outcome_marks_quorum_and_confidence() {
        let replies = vec![
            reply(
                "chatgpt",
                "Aplicar patch de segurança no servidor de borda imediatamente.",
            ),
            reply(
                "claude",
                "Aplicar patch de segurança no servidor de borda imediatamente agora.",
            ),
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
            connected
                .connect()
                .await
                .expect("nenhum cliente conectou ao pipe");

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
        let payload =
            r#"{"mission_id":"m-pipe","steps":[{"action":"restart","target":"spooler"}]}"#;
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

    #[test]
    fn classifies_readonly_and_mutating_tools() {
        // Readonly puro
        let text_ro = r#"
        Vamos inspecionar os arquivos:
        ```json
        {
            "steps": [
                { "tool": "filesystem.read", "input": { "path": "src/App.tsx" } },
                { "tool": "filesystem.list", "input": { "path": "src/components" } }
            ]
        }
        ```
        "#;
        let (ro, muta, steps) = classify_tools(text_ro);
        assert!(ro);
        assert!(!muta);
        assert_eq!(steps.len(), 2);

        // Mutating via SEARCH/REPLACE
        let text_sr = "<<<<<<< SEARCH: src/App.tsx\nlet a = 1;\n=======\nlet a = 2;\n>>>>>>> REPLACE";
        let (ro_sr, muta_sr, _) = classify_tools(text_sr);
        assert!(muta_sr);
        assert!(!ro_sr);

        // Mutating via JSON
        let text_write = r#"
        ```json
        {
            "tool": "filesystem.write",
            "input": { "path": "test.txt", "content": "hello" }
        }
        ```
        "#;
        let (_, muta_write, _) = classify_tools(text_write);
        assert!(muta_write);

        // Misto: read + write => mutating deve prevalecer
        let text_mixed = r#"
        ```json
        {
            "steps": [
                { "tool": "filesystem.read", "input": { "path": "test.txt" } },
                { "tool": "filesystem.write", "input": { "path": "test.txt", "content": "hi" } }
            ]
        }
        ```
        "#;
        let (ro_mixed, muta_mixed, _) = classify_tools(text_mixed);
        assert!(ro_mixed);
        assert!(muta_mixed);
    }

    #[test]
    fn extracts_context_log_from_search_replace_and_prompt() {
        let text = "<<<<<<< SEARCH: src/index.ts\nfoo\n=======\nbar\n>>>>>>> REPLACE\n<<<<<<< SEARCH: package.json\n1.0.0\n=======\n1.0.1\n>>>>>>> REPLACE";
        let ctx = extract_context_log_from_text(text, "Atualizar versao e funcao").unwrap();
        assert_eq!(ctx["intent"], "Atualizar versao e funcao");
        let files = ctx["files_inspected"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], "src/index.ts");
        assert_eq!(files[1], "package.json");
    }

    #[test]
    fn builds_rollback_checkpoint_envelope() {
        let payload = r#"{"kind":"rollback_checkpoint","checkpoint_id":"cp-123","mission_id":"m-rb"}"#;
        let (order, encoding, mission_id) = build_daemon_order(payload).unwrap();
        assert_eq!(encoding, "json");
        assert_eq!(mission_id.as_deref(), Some("m-rb"));
        assert_eq!(order["kind"], "rollback_checkpoint");
        assert_eq!(order["checkpoint_id"], "cp-123");
    }

    #[tokio::test]
    async fn preflight_workspace_index_falls_back_silently_when_pipe_missing() {
        // Sem daemon ouvindo, preflight deve retornar None sem causar pânico nem travar
        let index = preflight_workspace_index().await;
        assert!(index.is_none());
    }

    #[test]
    fn parses_raw_array_of_steps() {
        let text = r#"[
            {"tool": "filesystem.search", "input": {"query": "test"}},
            {"tool": "filesystem.stat", "input": {"path": "src/main.rs"}}
        ]"#;
        let (ro, muta, steps) = classify_tools(text);
        assert!(ro);
        assert!(!muta);
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn extracts_context_log_from_json_steps() {
        let text = r#"{
            "steps": [
                {"tool": "filesystem.write", "input": {"path": "src/new_file.rs", "content": "fn test() {}"}}
            ]
        }"#;
        let ctx = extract_context_log_from_text(text, "Criar novo arquivo").unwrap();
        assert_eq!(ctx["intent"], "Criar novo arquivo");
        let files = ctx["files_inspected"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "src/new_file.rs");
    }

    #[test]
    fn reads_mentioned_existing_file_in_prompt() {
        // Cargo.toml existe na raiz do workspace
        let prompt = r#"por favor revise o arquivo "Cargo.toml" para mim"#;
        let read = preflight_read_mentioned_files(prompt);
        assert!(read.is_some());
        let content = read.unwrap();
        assert!(content.contains("[CONTEÚDO DO ARQUIVO LOCAL (Cargo.toml)]"));
        assert!(content.contains("[package]") || content.contains("[workspace]"));
    }
}
