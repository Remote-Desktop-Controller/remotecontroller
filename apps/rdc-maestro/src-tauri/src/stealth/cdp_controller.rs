// RDC Maestro - Controlador Chrome DevTools Protocol (CDP)
// Gera payloads e comanda a injeção de eventos de baixo nível para que o motor
// Chromium marque as interações no DOM com `event.isTrusted = true`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseEventType {
    MouseMoved,
    MousePressed,
    MouseReleased,
    MouseWheel,
}

impl MouseEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MouseEventType::MouseMoved => "mouseMoved",
            MouseEventType::MousePressed => "mousePressed",
            MouseEventType::MouseReleased => "mouseReleased",
            MouseEventType::MouseWheel => "mouseWheel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

impl MouseButton {
    pub fn as_str(&self) -> &'static str {
        match self {
            MouseButton::None => "none",
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyEventType {
    KeyDown,
    KeyUp,
    RawKeyDown,
    Char,
}

impl KeyEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyEventType::KeyDown => "keyDown",
            KeyEventType::KeyUp => "keyUp",
            KeyEventType::RawKeyDown => "rawKeyDown",
            KeyEventType::Char => "char",
        }
    }
}

/// Constrói o payload JSON exato do comando CDP `Input.dispatchMouseEvent`
pub fn build_mouse_event(
    event_type: MouseEventType,
    x: f64,
    y: f64,
    button: MouseButton,
    click_count: i32,
) -> Value {
    json!({
        "type": event_type.as_str(),
        "x": x,
        "y": y,
        "button": button.as_str(),
        "clickCount": click_count
    })
}

/// Constrói o payload JSON exato do comando CDP `Input.dispatchKeyEvent`
pub fn build_key_event(
    event_type: KeyEventType,
    text: Option<&str>,
    code: Option<&str>,
    virtual_key_code: Option<i32>,
) -> Value {
    let mut payload = json!({
        "type": event_type.as_str(),
    });

    if let Some(t) = text {
        payload["text"] = json!(t);
        payload["unmodifiedText"] = json!(t);
    }
    if let Some(c) = code {
        payload["code"] = json!(c);
    }
    if let Some(vk) = virtual_key_code {
        payload["windowsVirtualKeyCode"] = json!(vk);
    }

    payload
}

/// Controlador de entrada furtiva de hardware
pub struct CdpController;

impl CdpController {
    /// Formata a chamada JSON-RPC do método CDP
    pub fn format_cdp_call(id: u64, method: &str, params: Value) -> String {
        json!({
            "id": id,
            "method": method,
            "params": params
        })
        .to_string()
    }
}
