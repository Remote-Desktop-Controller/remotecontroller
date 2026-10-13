//! Testes para o módulo CDP controller e despacho de eventos com isTrusted = true

use rdc_maestro_lib::stealth::cdp_controller::{
    build_mouse_event, build_key_event, MouseEventType, KeyEventType, MouseButton,
};

#[test]
fn builds_valid_cdp_mouse_move_payload() {
    let payload = build_mouse_event(MouseEventType::MouseMoved, 450.5, 320.0, MouseButton::None, 0);

    assert_eq!(payload["type"], "mouseMoved");
    assert_eq!(payload["x"], 450.5);
    assert_eq!(payload["y"], 320.0);
    assert_eq!(payload["button"], "none");
    assert_eq!(payload["clickCount"], 0);
}

#[test]
fn builds_valid_cdp_mouse_click_payloads() {
    let press = build_mouse_event(MouseEventType::MousePressed, 120.0, 85.0, MouseButton::Left, 1);
    assert_eq!(press["type"], "mousePressed");
    assert_eq!(press["button"], "left");
    assert_eq!(press["clickCount"], 1);

    let release = build_mouse_event(MouseEventType::MouseReleased, 120.0, 85.0, MouseButton::Left, 1);
    assert_eq!(release["type"], "mouseReleased");
    assert_eq!(release["button"], "left");
    assert_eq!(release["clickCount"], 1);
}

#[test]
fn builds_valid_cdp_keyboard_event_payload() {
    let key_down = build_key_event(KeyEventType::KeyDown, Some("a"), Some("KeyA"), Some(65));
    assert_eq!(key_down["type"], "keyDown");
    assert_eq!(key_down["text"], "a");
    assert_eq!(key_down["code"], "KeyA");
    assert_eq!(key_down["windowsVirtualKeyCode"], 65);

    let key_up = build_key_event(KeyEventType::KeyUp, None, Some("KeyA"), Some(65));
    assert_eq!(key_up["type"], "keyUp");
    assert_eq!(key_up["code"], "KeyA");
}
