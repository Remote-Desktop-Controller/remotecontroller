//! Secret sanitization and redactor pipeline.
//!
//! Masks sensitive credentials (OpenAI, Anthropic, Bearer tokens, GitHub/GitLab tokens,
//! Slack tokens, SSH private keys, and credential key-values) before persisting audit logs
//! or streaming events across IPC channels.

use regex::Regex;
use std::sync::LazyLock;

/// Redaction mask substituted for sensitive contents.
pub const REDACTION_MASK: &str = "[REDACTED]";

/// Pre-compiled regular expressions for secret detection.
static REDACTION_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // SSH / PEM Private Keys (handles PKCS#1, PKCS#8, OpenSSH, EC, etc.)
        (
            Regex::new(r"(?s)-----BEGIN (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----.*?-----END (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----").unwrap(),
            "[REDACTED_PRIVATE_KEY]",
        ),
        // Anthropic API keys (must precede generic sk- to prevent partial matching)
        (
            Regex::new(r"\bsk-ant-[a-zA-Z0-9_\-]{20,}\b").unwrap(),
            "[REDACTED_ANTHROPIC_KEY]",
        ),
        // OpenAI API keys (legacy, modern project keys, and service account keys)
        (
            Regex::new(r"\bsk-(?:proj-|svcacct-|admin-)?[a-zA-Z0-9_\-]{20,}\b").unwrap(),
            "[REDACTED_OPENAI_KEY]",
        ),
        // Bearer Authentication Tokens (including standard Base64/Base64URL padding and chars)
        (
            Regex::new(r"(?i)\bBearer\s+[a-zA-Z0-9_\-\.\+/=]{20,}").unwrap(),
            "Bearer [REDACTED_TOKEN]",
        ),
        // GitHub Personal Access & OAuth Tokens (legacy and modern fine-grained PATs)
        (
            Regex::new(r"\b(?:ghp|gho|ghu|ghs|ghr|github_pat)_[a-zA-Z0-9_]{20,}\b").unwrap(),
            "[REDACTED_GITHUB_TOKEN]",
        ),
        // AWS Access Key IDs
        (
            Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b").unwrap(),
            "[REDACTED_AWS_KEY]",
        ),
        // GitLab Personal Access Tokens
        (
            Regex::new(r"\bglpat-[a-zA-Z0-9_\-]{20,}\b").unwrap(),
            "[REDACTED_GITLAB_TOKEN]",
        ),
        // Slack tokens
        (
            Regex::new(r"\bxox[baprs]-[a-zA-Z0-9_\-]{10,}\b").unwrap(),
            "[REDACTED_SLACK_TOKEN]",
        ),
        // Password / Token / Secret key-value assignments (supporting quoted key names)
        (
            Regex::new(r#"(?i)["']?\b(api[_-]?key|secret|password|access[_-]?token|auth[_-]?token)["']?\s*([:=])\s*["']?([a-zA-Z0-9_\-\.\+/=]{8,})["']?"#).unwrap(),
            "$1$2[REDACTED_SECRET]",
        ),
    ]
});

/// Checks whether an object key name implies a sensitive secret.
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("password")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("private_key")
        || lower.contains("access_key")
        || lower.contains("auth_key")
}

/// Redactor engine applying regex sanitization rules.
#[derive(Debug, Default, Clone)]
pub struct Redactor;

impl Redactor {
    /// Constructs a new redactor instance.
    pub fn new() -> Self {
        Self
    }

    /// Redacts all sensitive token patterns from a string slice.
    pub fn redact(&self, input: &str) -> String {
        let mut sanitized = input.to_string();

        for (regex, replacement) in REDACTION_RULES.iter() {
            sanitized = regex.replace_all(&sanitized, *replacement).into_owned();
        }

        sanitized
    }

    /// Recursively redacts sensitive strings within a serde JSON Value.
    pub fn redact_json(&self, val: &serde_json::Value) -> serde_json::Value {
        match val {
            serde_json::Value::String(s) => serde_json::Value::String(self.redact(s)),
            serde_json::Value::Array(arr) => {
                let cleaned = arr.iter().map(|item| self.redact_json(item)).collect();
                serde_json::Value::Array(cleaned)
            }
            serde_json::Value::Object(map) => {
                let mut cleaned = serde_json::Map::new();
                for (k, v) in map {
                    if is_sensitive_key(k) {
                        match v {
                            serde_json::Value::String(_) => {
                                cleaned.insert(
                                    k.clone(),
                                    serde_json::Value::String(REDACTION_MASK.to_string()),
                                );
                            }
                            _ => {
                                cleaned.insert(k.clone(), self.redact_json(v));
                            }
                        }
                    } else {
                        cleaned.insert(k.clone(), self.redact_json(v));
                    }
                }
                serde_json::Value::Object(cleaned)
            }
            other => other.clone(),
        }
    }
}

/// Convenience global helper to sanitize text.
pub fn redact_text(input: &str) -> String {
    Redactor::new().redact(input)
}

/// Convenience global helper to sanitize a JSON Value.
pub fn redact_value(value: &serde_json::Value) -> serde_json::Value {
    Redactor::new().redact_json(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_openai_key() {
        let text = "Using openai key: sk-proj-1234567890abcdef1234567890 and legacy sk-abc12345678901234567890";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("sk-proj-"));
        assert!(!cleaned.contains("sk-abc"));
        assert!(cleaned.contains("[REDACTED_OPENAI_KEY]"));
    }

    #[test]
    fn test_redact_anthropic_key() {
        let text =
            "Configured ANTHROPIC_API_KEY=sk-ant-api03-abcdef1234567890abcdef1234567890 in env";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("sk-ant-"));
        assert!(cleaned.contains("[REDACTED_ANTHROPIC_KEY]"));
    }

    #[test]
    fn test_redact_bearer_token() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abcdefgh1234567890";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("eyJhbGciOi"));
        assert!(cleaned.contains("Bearer [REDACTED_TOKEN]"));
    }

    #[test]
    fn test_redact_ssh_private_key() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAABlwAAAAdzc2gtcn\n-----END OPENSSH PRIVATE KEY-----";
        let text = format!("Host key is:\n{key}\nPlease save it.");
        let cleaned = redact_text(&text);
        assert!(!cleaned.contains("b3BlbnNzaC1rZXktdjE"));
        assert!(cleaned.contains("[REDACTED_PRIVATE_KEY]"));
    }

    #[test]
    fn test_redact_github_and_gitlab() {
        let text = "GH token: ghp_123456789012345678901234567890123456, GL token: glpat-abcdef1234567890abcdef";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("ghp_1234"));
        assert!(!cleaned.contains("glpat-abc"));
        assert!(cleaned.contains("[REDACTED_GITHUB_TOKEN]"));
        assert!(cleaned.contains("[REDACTED_GITLAB_TOKEN]"));
    }

    #[test]
    fn test_redact_json_structure() {
        let json = serde_json::json!({
            "command": "curl -H 'Authorization: Bearer mytoken12345678901234567890'",
            "env": {
                "API_KEY": "sk-proj-9876543210abcdef9876543210",
                "PASSWORD": "mysecretpassword123"
            },
            "safe_counter": 42
        });

        let cleaned = redact_value(&json);
        let s = cleaned.to_string();
        assert!(!s.contains("sk-proj-"));
        assert!(!s.contains("mytoken1234"));
        assert!(
            !s.contains("mysecretpassword123"),
            "Secret password value MUST be redacted"
        );
        assert_eq!(cleaned["safe_counter"], 42);
    }

    #[test]
    fn test_redact_pkcs8_unadorned_private_key() {
        let key = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDabc123\n-----END PRIVATE KEY-----";
        let text = format!("Server key:\n{key}");
        let cleaned = redact_text(&text);
        assert!(!cleaned.contains("MIIEvgIBADAN"));
        assert!(cleaned.contains("[REDACTED_PRIVATE_KEY]"));
    }

    #[test]
    fn test_redact_base64_bearer_token_with_symbols() {
        let text = "Authorization: Bearer dGVzdCthYmMvMTIzNDU2Nzg5MDEyMzQ1Njc4OTA=";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("dGVzdCthYmM"));
        assert!(cleaned.contains("Bearer [REDACTED_TOKEN]"));
    }

    #[test]
    fn test_redact_github_fine_grained_pat() {
        let text = "Token: github_pat_11A6W3D4Q0abc123def456ghi789jkl012mno345pqr678";
        let cleaned = redact_text(text);
        assert!(!cleaned.contains("github_pat_11A6W"));
        assert!(cleaned.contains("[REDACTED_GITHUB_TOKEN]"));
    }
}
