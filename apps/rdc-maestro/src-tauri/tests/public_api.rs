//! Consumidor externo da biblioteca do Maestro: garante que a superfície pública usada
//! pelo binário Tauri (motor blackboard, ponte com o daemon e helpers) continua linkável
//! e estável fora do crate — o mesmo caminho que a UI e os testes percorrem.
use rdc_maestro_lib::orchestrator::{
    DebateOutcome, MaestroEngine, PlanExecutionReceipt, daemon_pipe_candidates, iso8601_utc,
    new_mission_id,
};
use rdc_maestro_lib::webview_manager::WebviewManager;

#[test]
fn exposes_the_blackboard_engine_surface() {
    let engine = MaestroEngine::new();
    assert!(!engine.reply_sender().is_closed());

    // Os tipos devolvidos pelo motor precisam ser serializáveis para o IPC do Tauri.
    let outcome = DebateOutcome {
        mission_id: "m-smoke".to_string(),
        prompt: "ping".to_string(),
        status: "no_reply".to_string(),
        consensus: String::new(),
        convergences: Vec::new(),
        divergences: Vec::new(),
        replies: Vec::new(),
        missing: Vec::new(),
        quorum_met: false,
        coverage: 0,
        confidence: 0.0,
        elapsed_ms: 0,
        started_at: 0,
        finished_at: 0,
    };
    let encoded = serde_json::to_value(&outcome).expect("DebateOutcome precisa serializar");
    assert_eq!(encoded["mission_id"], "m-smoke");

    let receipt = PlanExecutionReceipt {
        endpoint: r"\\.\pipe\rdc-agent-maestro".to_string(),
        mission_id: None,
        encoding: "text".to_string(),
        bytes_written: 10,
        dispatched_at: 0,
        acknowledged: false,
        response: None,
    };
    assert!(serde_json::to_string(&receipt).is_ok());
}

#[test]
fn exposes_daemon_channel_and_provider_catalog() {
    let pipes = daemon_pipe_candidates();
    assert!(
        pipes
            .iter()
            .any(|pipe| pipe == r"\\.\pipe\rdc-agent-maestro")
    );
    assert!(WebviewManager::find_config("chatgpt").is_some());
    assert!(WebviewManager::find_config("inexistente").is_none());
    assert!(new_mission_id().starts_with("m-"));
    assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
}
