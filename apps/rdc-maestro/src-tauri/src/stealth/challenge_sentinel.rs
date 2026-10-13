// RDC Maestro - Sentinela de Desafios e Auto-Exposição Human-in-the-Loop
// Monitora a presença de desafios da Cloudflare, Google reCAPTCHA, Geetest e Arkose.
// Quando um desafio é detectado, a janela oculta é imediatamente exibida para o operador resolver.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeType {
    CloudflareTurnstile,
    GoogleRecaptcha,
    GeetestSlider,
    ArkoseFunCaptcha,
    GenericChallenge,
}

impl ChallengeType {
    pub fn description(&self) -> &'static str {
        match self {
            ChallengeType::CloudflareTurnstile => "Cloudflare Turnstile Challenge",
            ChallengeType::GoogleRecaptcha => "Google reCAPTCHA",
            ChallengeType::GeetestSlider => "Geetest Slider Challenge",
            ChallengeType::ArkoseFunCaptcha => "Arkose FunCaptcha",
            ChallengeType::GenericChallenge => "Verificação de Segurança Genérica",
        }
    }
}

/// Payload emitido para a UI quando um desafio é detectado
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeNotification {
    pub provider: String,
    pub challenge: ChallengeType,
    pub description: String,
    pub timestamp: u64,
}

/// Analisa o HTML/DOM retornado pela WebView para detectar marcadores de CAPTCHAs e WAFs
pub fn detect_challenge_in_html(html: &str) -> Option<ChallengeType> {
    let lower = html.to_lowercase();

    if lower.contains("challenges.cloudflare.com") || lower.contains("cf-turnstile") || lower.contains("/challenge-platform/") {
        return Some(ChallengeType::CloudflareTurnstile);
    }

    if lower.contains("google.com/recaptcha") || lower.contains("recaptcha/api") || lower.contains("g-recaptcha") {
        return Some(ChallengeType::GoogleRecaptcha);
    }

    if lower.contains("geetest_holder") || lower.contains("geetest_wind") || lower.contains("gt_holder") {
        return Some(ChallengeType::GeetestSlider);
    }

    if lower.contains("arkoselabs") || lower.contains("arkose-box") || lower.contains("funcaptcha") {
        return Some(ChallengeType::ArkoseFunCaptcha);
    }

    if lower.contains("security check") || lower.contains("human verification") || lower.contains("verificação de segurança") {
        return Some(ChallengeType::GenericChallenge);
    }

    None
}

/// Script JavaScript compacto injetado para detecção instantânea de CAPTCHA no DOM
pub const DOM_CHALLENGE_DETECTOR_SCRIPT: &str = r#"(function() {
  const isTurnstile = Boolean(document.querySelector('iframe[src*="challenges.cloudflare.com"], .cf-turnstile'));
  const isRecaptcha = Boolean(document.querySelector('iframe[src*="recaptcha"], .g-recaptcha'));
  const isGeetest = Boolean(document.querySelector('.geetest_holder, .geetest_wind'));
  const isArkose = Boolean(document.querySelector('iframe[src*="arkoselabs"], .arkose-box'));

  if (isTurnstile) return "cloudflare_turnstile";
  if (isRecaptcha) return "google_recaptcha";
  if (isGeetest) return "geetest_slider";
  if (isArkose) return "arkose_funcaptcha";
  return "";
})()"#;
