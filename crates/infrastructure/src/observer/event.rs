//! Audit event schema and serialization for real-time observability.
//!
//! Event contract conforms to Master Directive:
//! ```json
//! {
//!   "timestamp": 1775836800000,
//!   "event_type": "TOOL_INVOKED" | "FILE_MODIFIED" | "COMMAND_EXECUTED",
//!   "target": "src/main.rs",
//!   "diff": { "added": 12, "removed": 4 },
//!   "execution_time_ms": 42,
//!   "status": "SUCCESS"
//! }
//! ```

use crate::telemetry::redactor::redact_text;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Type of audit events emitted across the IPC pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuditEventType {
    ToolInvoked,
    FileModified,
    CommandExecuted,
}

impl std::fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolInvoked => write!(f, "TOOL_INVOKED"),
            Self::FileModified => write!(f, "FILE_MODIFIED"),
            Self::CommandExecuted => write!(f, "COMMAND_EXECUTED"),
        }
    }
}

/// Execution status of the observed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuditStatus {
    Success,
    Failed,
    Cancelled,
}

impl std::fmt::Display for AuditStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "SUCCESS"),
            Self::Failed => write!(f, "FAILED"),
            Self::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

/// Diff summary metrics embedded in file modification events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditDiffMetrics {
    pub added: usize,
    pub removed: usize,
}

/// Structured audit event emitted in real-time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: u64,
    pub event_type: AuditEventType,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<AuditDiffMetrics>,
    pub execution_time_ms: u64,
    pub status: AuditStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl AuditEvent {
    /// Constructs a `FILE_MODIFIED` event with exact diff metrics.
    pub fn file_modified(
        target: impl Into<String>,
        added: usize,
        removed: usize,
        execution_time_ms: u64,
        status: AuditStatus,
    ) -> Self {
        Self {
            timestamp: current_timestamp_ms(),
            event_type: AuditEventType::FileModified,
            target: target.into(),
            diff: Some(AuditDiffMetrics { added, removed }),
            execution_time_ms,
            status,
            details: None,
        }
    }

    /// Constructs a `TOOL_INVOKED` event with sanitized parameters.
    pub fn tool_invoked(
        tool_name: impl Into<String>,
        execution_time_ms: u64,
        status: AuditStatus,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            timestamp: current_timestamp_ms(),
            event_type: AuditEventType::ToolInvoked,
            target: tool_name.into(),
            diff: None,
            execution_time_ms,
            status,
            details: details.map(|d| crate::telemetry::redactor::redact_value(&d)),
        }
    }

    /// Constructs a `COMMAND_EXECUTED` event with command string sanitization.
    pub fn command_executed(
        command: &str,
        execution_time_ms: u64,
        status: AuditStatus,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            timestamp: current_timestamp_ms(),
            event_type: AuditEventType::CommandExecuted,
            target: redact_text(command),
            diff: None,
            execution_time_ms,
            status,
            details: details.map(|d| crate::telemetry::redactor::redact_value(&d)),
        }
    }

    /// Serializes this audit event as a single JSON line.
    pub fn to_json_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Formats this event for clean terminal display.
    pub fn format_display(&self, use_ansi: bool) -> String {
        let (color_reset, color_type, color_green, color_red, color_status) = if use_ansi {
            let status_col = match self.status {
                AuditStatus::Success => "\x1b[32m",
                AuditStatus::Failed => "\x1b[31m",
                AuditStatus::Cancelled => "\x1b[33m",
            };
            (
                "\x1b[0m",
                "\x1b[1;36m", // Cyan bold
                "\x1b[32m",   // Green
                "\x1b[31m",   // Red
                status_col,
            )
        } else {
            ("", "", "", "", "")
        };

        let diff_str = match &self.diff {
            Some(d) => format!(
                " [{color_green}+{}{color_reset}/{color_red}-{}{color_reset}]",
                d.added, d.removed
            ),
            None => String::new(),
        };

        format!(
            "[{color_type}{:<16}{color_reset}] target='{}'{} ({}ms) -> {color_status}{}{color_reset}",
            self.event_type, self.target, diff_str, self.execution_time_ms, self.status
        )
    }
}

pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_json_schema() {
        let event = AuditEvent {
            timestamp: 1775836800000,
            event_type: AuditEventType::FileModified,
            target: "src/main.rs".into(),
            diff: Some(AuditDiffMetrics {
                added: 12,
                removed: 4,
            }),
            execution_time_ms: 42,
            status: AuditStatus::Success,
            details: None,
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["timestamp"], 1775836800000u64);
        assert_eq!(json["event_type"], "FILE_MODIFIED");
        assert_eq!(json["target"], "src/main.rs");
        assert_eq!(json["diff"]["added"], 12);
        assert_eq!(json["diff"]["removed"], 4);
        assert_eq!(json["execution_time_ms"], 42);
        assert_eq!(json["status"], "SUCCESS");
    }

    #[test]
    fn test_command_executed_redaction() {
        let cmd = "curl -H 'Authorization: Bearer my-secret-token-12345678901234567890' https://api.openai.com";
        let event = AuditEvent::command_executed(cmd, 15, AuditStatus::Success, None);

        assert_eq!(event.event_type, AuditEventType::CommandExecuted);
        assert!(!event.target.contains("my-secret-token"));
        assert!(event.target.contains("Bearer [REDACTED_TOKEN]"));
    }

    #[test]
    fn test_format_display() {
        let event = AuditEvent::file_modified("crates/storage.rs", 10, 2, 55, AuditStatus::Success);
        let formatted = event.format_display(false);
        assert!(formatted.contains("FILE_MODIFIED"));
        assert!(formatted.contains("crates/storage.rs"));
        assert!(formatted.contains("+10/-2"));
        assert!(formatted.contains("SUCCESS"));
    }
}
