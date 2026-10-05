use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn now() -> i64 {
    (runtime_protocol::now_ms() / 1000) as i64
}
pub fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn hash(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}
pub fn csrf(secret: &str) -> String {
    hash(&format!("csrf:{secret}"))
}
pub fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
pub fn password_hash(password: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|_| "Falha ao proteger senha".into())
}
pub fn password_matches(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        && constant_eq(&hash(verifier), challenge)
}
pub fn redirect_allowed(uri: &str, allowlist: &[String], allow_loopback: bool) -> bool {
    if uri.len() > 2048
        || uri
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return false;
    }
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    (url.scheme() == "https" && allowlist.iter().any(|v| v == uri))
        || (allow_loopback && local && url.scheme() == "http")
}
pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_rfc7636_vector_rejects_wrong_and_short_verifiers() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(&"x".repeat(43), challenge));
        assert!(!pkce_matches("tiny", challenge));
    }
    #[test]
    fn redirects_require_exact_allowlist_and_no_credentials_or_fragment() {
        let allowed: Vec<String> =
            vec!["https://chatgpt.com/connector_platform/oauth_redirect".into()];
        assert!(redirect_allowed(&allowed[0], &allowed, false));
        assert!(!redirect_allowed(
            "https://chatgpt.com.evil.test/connector_platform/oauth_redirect",
            &allowed,
            false
        ));
        assert!(!redirect_allowed(
            "https://chatgpt.com/connector_platform/oauth_redirect?evil=1",
            &allowed,
            false
        ));
        assert!(!redirect_allowed(
            "http://127.0.0.1:1234/callback",
            &[],
            false
        ));
        assert!(redirect_allowed(
            "http://127.0.0.1:1234/callback",
            &[],
            true
        ));
        assert!(!redirect_allowed(
            "http://127.0.0.1:1234/callback#fragment",
            &[],
            true
        ));
        assert!(!redirect_allowed(
            "http://user@127.0.0.1/callback",
            &[],
            true
        ));
    }
}
