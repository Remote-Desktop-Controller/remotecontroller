// RDC Maestro - Núcleo da Aplicação Desktop Multi-Agente
// Coordena o ciclo de vida do Tauri v2, comandos IPC e janelas de sessões web.
//
// Fase 1: sessões web headless (ChatGPT, Claude, Gemini, DeepSeek, Qwen).
// Fase 3: motor blackboard de debate multi-agente (`orchestrator::MaestroEngine`).
// Fase 4: ponte IPC com o daemon local via Named Pipe nativo.

pub mod orchestrator;
pub mod stealth;
pub mod webview_manager;

// O harness unitário da biblioteca (`cargo test --lib`) não recebe o `rustc-link-arg-bins`
// do embed-resource e, sem o manifesto Common-Controls v6, o Windows aborta o processo na
// inicialização (a pilha tao/muda importa `TaskDialogIndirect` do comctl32 v5). Este link
// vale somente quando a própria biblioteca é compilada em modo de teste, de modo que
// binários e recursos de produção continuam com uma única cópia do manifesto.
#[cfg(all(test, windows, target_env = "gnu"))]
#[link(name = "resource", kind = "static")]
unsafe extern "C" {}

use orchestrator::{DebateOutcome, MaestroEngine, MissionRecord, PlanExecutionReceipt};
use tauri::{AppHandle, State, WindowEvent};
use tracing::info;
use webview_manager::{ProviderStatus, WebviewManager};

// --------------------------------------------------------------------------------------
// Fase 1 - Sessões web headless
// --------------------------------------------------------------------------------------

#[tauri::command]
async fn open_provider_login(
    app: AppHandle,
    manager: State<'_, WebviewManager>,
    provider: String,
) -> Result<ProviderStatus, String> {
    manager.open_login(&app, &provider).await
}

#[tauri::command]
async fn hide_provider_window(
    app: AppHandle,
    manager: State<'_, WebviewManager>,
    provider: String,
) -> Result<ProviderStatus, String> {
    manager.hide_window(&app, &provider).await
}

#[tauri::command]
async fn show_provider_window(
    app: AppHandle,
    manager: State<'_, WebviewManager>,
    provider: String,
) -> Result<ProviderStatus, String> {
    manager.show_window(&app, &provider).await
}

#[tauri::command]
async fn set_provider_connected(
    app: AppHandle,
    manager: State<'_, WebviewManager>,
    provider: String,
) -> Result<ProviderStatus, String> {
    manager.set_connected(&app, &provider).await
}

#[tauri::command]
async fn get_all_providers_status(
    manager: State<'_, WebviewManager>,
) -> Result<Vec<ProviderStatus>, String> {
    Ok(manager.get_all_statuses().await)
}

// --------------------------------------------------------------------------------------
// Fase 3 - Motor blackboard: entrada de respostas e ciclo de debate
// --------------------------------------------------------------------------------------

/// Rota IPC que o JS das WebViews usa para devolver o texto lido do último turno do agente.
///
/// `partial = true` sinaliza streaming em andamento (apenas evento de progresso);
/// `partial = false` consolida a resposta no canal de blackboard do motor.
#[tauri::command]
async fn maestro_receive_reply(
    app: AppHandle,
    engine: State<'_, MaestroEngine>,
    provider: String,
    text: String,
    mission: Option<String>,
    partial: Option<bool>,
) -> Result<bool, String> {
    engine
        .submit_reply(
            &app,
            &provider,
            &text,
            mission.as_deref(),
            partial.unwrap_or(false),
        )
        .await
}

/// Gatilho inicial da UI: injeta o prompt nas sessões ocultas dos provedores, aguarda as respostas no
/// canal assíncrono e devolve o documento de consenso multi-agente.
#[tauri::command]
async fn maestro_start_mission(
    app: AppHandle,
    engine: State<'_, MaestroEngine>,
    prompt: String,
) -> Result<DebateOutcome, String> {
    info!("Missão recebida da UI. Iniciando ciclo de debate multi-agente.");
    engine.run_debate_cycle(&app, &prompt).await
}

/// Fase 4: envia a ordem física aprovada pelo operador ao `local-daemon` via Named Pipe.
#[tauri::command]
async fn maestro_execute_plan(
    app: AppHandle,
    engine: State<'_, MaestroEngine>,
    payload: String,
) -> Result<PlanExecutionReceipt, String> {
    orchestrator::emit_log(
        &app,
        orchestrator::LogLevel::Info,
        "Despachando ordem física aprovada para o daemon local...",
    );

    let result = engine.execute_approved_plan(&payload).await;

    match &result {
        Ok(receipt) => {
            orchestrator::emit_log(
                &app,
                orchestrator::LogLevel::Success,
                format!(
                    "Ordem despachada via {} ({} bytes, ack: {}).",
                    receipt.endpoint, receipt.bytes_written, receipt.acknowledged
                ),
            );
            let _ = tauri::Emitter::emit(&app, "maestro-plan-dispatched", receipt);
        }
        Err(err) => {
            orchestrator::emit_log(
                &app,
                orchestrator::LogLevel::Error,
                format!("Falha ao despachar ordem física: {err}"),
            );
        }
    }

    result
}

/// Reidratação da UI: devolve o último consenso produzido, se houver.
#[tauri::command]
async fn maestro_get_last_consensus(
    engine: State<'_, MaestroEngine>,
) -> Result<Option<DebateOutcome>, String> {
    Ok(engine.last_outcome().await)
}

/// Consulta o registro de uma missão específica (auditoria do blackboard).
#[tauri::command]
async fn maestro_get_mission(
    engine: State<'_, MaestroEngine>,
    mission_id: String,
) -> Result<Option<MissionRecord>, String> {
    Ok(engine.mission(&mission_id).await)
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("info,rdc_maestro=debug")
        .init();

    info!("Iniciando RDC Maestro v0.1.0...");

    let webview_manager = WebviewManager::new();
    let maestro_engine = MaestroEngine::new();

    tauri::Builder::default()
        .manage(webview_manager)
        .manage(maestro_engine)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label.starts_with("provider-") {
                    api.prevent_close();
                    let _ = window.hide();
                    info!("Evento de fechar interceptado na janela [{}]. Mantendo sessão em background (oculta).", label);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_provider_login,
            hide_provider_window,
            show_provider_window,
            set_provider_connected,
            get_all_providers_status,
            maestro_receive_reply,
            maestro_start_mission,
            maestro_execute_plan,
            maestro_get_last_consensus,
            maestro_get_mission
        ])
        .run(tauri::generate_context!())
        .expect("Erro fatal ao executar a aplicação RDC Maestro");
}
