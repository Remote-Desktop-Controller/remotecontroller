//! Testes para o sentinela de desafios e detecção de CAPTCHAs

use rdc_maestro_lib::stealth::challenge_sentinel::{ChallengeType, detect_challenge_in_html};

#[test]
fn detects_cloudflare_turnstile_iframe() {
    let html = r#"
        <div class="main-body">
            <iframe src="https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/b/turnstile/if/ov2/av0/rcv0/0/sample"></iframe>
        </div>
    "#;

    let challenge = detect_challenge_in_html(html);
    assert!(challenge.is_some());
    assert_eq!(challenge.unwrap(), ChallengeType::CloudflareTurnstile);
}

#[test]
fn detects_google_recaptcha_iframe() {
    let html = r#"
        <div id="captcha-box">
            <iframe src="https://www.google.com/recaptcha/api2/anchor?ar=1&amp;k=sample"></iframe>
        </div>
    "#;

    let challenge = detect_challenge_in_html(html);
    assert!(challenge.is_some());
    assert_eq!(challenge.unwrap(), ChallengeType::GoogleRecaptcha);
}

#[test]
fn detects_geetest_slider_challenge() {
    let html = r#"
        <div class="geetest_holder geetest_wind">
            <div class="geetest_canvas_img"></div>
        </div>
    "#;

    let challenge = detect_challenge_in_html(html);
    assert!(challenge.is_some());
    assert_eq!(challenge.unwrap(), ChallengeType::GeetestSlider);
}

#[test]
fn reports_none_when_page_is_clean() {
    let html = r#"
        <div class="chat-container">
            <textarea id="prompt-textarea" placeholder="Message ChatGPT..."></textarea>
            <button data-testid="send-button">Send</button>
        </div>
    "#;

    let challenge = detect_challenge_in_html(html);
    assert!(challenge.is_none());
}
