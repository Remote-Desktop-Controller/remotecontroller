//! Versioned wire DTOs. Changes to this contract require protocol negotiation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const VERSION: u16 = 2;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const HANDSHAKE_MAX_FRAME: usize = 1024;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub trace_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
    pub operation_id: Uuid,
    pub deadline: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}
impl Metadata {
    pub fn new(workspace_id: Option<Uuid>, timeout_ms: u64) -> Self {
        Self {
            protocol_version: VERSION,
            request_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            workspace_id,
            task_id: None,
            session_id: None,
            operation_id: Uuid::new_v4(),
            deadline: now_ms().saturating_add(timeout_ms),
            timeout_ms: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub meta: Metadata,
    pub payload: Request,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Shutdown,
    Catalog,
    Heartbeat,
    Execute { tool: String, input: Value },
    Cancel { operation_id: Uuid },
    GetOperation { operation_id: Uuid },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub risk: String,
    pub input_schema: Value,
    pub output_schema: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub meta: Metadata,
    pub payload: Result<Value, ErrorEnvelope>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    VersionMismatch,
    Unauthorized,
    InvalidRequest,
    PolicyDenied,
    NotFound,
    Busy,
    Cancelled,
    TimedOut,
    Conflict,
    Storage,
    Tool,
    Disconnected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub meta: Metadata,
    pub completed: usize,
    pub total: usize,
    pub message: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Proof(pub [u8; 32]);
impl std::fmt::Debug for Proof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Proof(<redacted>)")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Frame {
    /// Legacy frame decoded only for an explicit version rejection.
    Hello {
        protocol_version: u16,
        secret: Vec<u8>,
    },
    ClientHello {
        protocol_version: u16,
        client_nonce: [u8; 32],
    },
    ServerChallenge {
        protocol_version: u16,
        workspace_id: Uuid,
        server_nonce: [u8; 32],
        server_proof: Proof,
    },
    ClientProof {
        client_proof: Proof,
    },
    Welcome {
        protocol_version: u16,
        workspace_id: Uuid,
    },
    Request(RequestEnvelope),
    Response(ResponseEnvelope),
    Event(EventEnvelope),
    Error(ErrorEnvelope),
}
impl ErrorEnvelope {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code == ErrorCode::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v1_metadata_without_session_remains_readable() {
        let mut legacy = serde_json::to_value(Metadata::new(None, 1000)).unwrap();
        legacy.as_object_mut().unwrap().remove("session_id");
        let restored: Metadata = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.session_id, None);
    }
}
