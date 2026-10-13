//! Teste de conformidade de sanitização e furtividade do preload.js
//! Garante que nenhum identificador proprietário vaza no escopo global `window`
//! e que o título da página nunca é alterado por marcadores de bot.

use rdc_maestro_lib::webview_manager::PRELOAD_SCRIPT;

#[test]
fn preload_does_not_pollute_window_with_maestro_identifiers() {
    let script = PRELOAD_SCRIPT;

    // Não pode conter atribuições globais a window com prefixo __maestro
    assert!(
        !script.contains("window.__maestro ="),
        "PRELOAD_SCRIPT vaza window.__maestro"
    );
    assert!(
        !script.contains("window.__maestro_send_prompt ="),
        "PRELOAD_SCRIPT vaza window.__maestro_send_prompt"
    );
    assert!(
        !script.contains("window.__maestro_check_auth ="),
        "PRELOAD_SCRIPT vaza window.__maestro_check_auth"
    );
    assert!(
        !script.contains("window.__maestro_on_reply ="),
        "PRELOAD_SCRIPT vaza window.__maestro_on_reply"
    );
    assert!(
        !script.contains("window.__maestro_extract_response ="),
        "PRELOAD_SCRIPT vaza window.__maestro_extract_response"
    );
    assert!(
        !script.contains("window.__maestro_capture_status ="),
        "PRELOAD_SCRIPT vaza window.__maestro_capture_status"
    );
    assert!(
        !script.contains("window.__MAESTRO_PRELOAD_INITIALIZED__ ="),
        "PRELOAD_SCRIPT vaza window.__MAESTRO_PRELOAD_INITIALIZED__"
    );
}

#[test]
fn preload_does_not_modify_document_title_with_bot_markers() {
    let script = PRELOAD_SCRIPT;

    // Jamais alterar document.title com marcador óbvio de bot
    assert!(
        !script.contains("[MAESTRO_AUTH_OK]"),
        "PRELOAD_SCRIPT insere marcador [MAESTRO_AUTH_OK] no document.title"
    );
}

#[test]
fn preload_does_not_manually_expose_tauri_globals() {
    let script = PRELOAD_SCRIPT;

    // Jamais definir manualmente window.__TAURI__
    assert!(
        !script.contains("window.__TAURI__ = {"),
        "PRELOAD_SCRIPT vaza window.__TAURI__ = {{ ... }}"
    );
}

#[test]
fn preload_uses_web_messaging_listener_for_commands() {
    let script = PRELOAD_SCRIPT;

    // Deve escutar comandos via message event padrão HTML5
    assert!(
        script.contains("addEventListener(\"message\"") || script.contains("addEventListener('message'"),
        "PRELOAD_SCRIPT deve escutar via window.addEventListener('message', ...)"
    );
}
