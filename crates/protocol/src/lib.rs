//! Versioned wire DTOs. Changes to this contract require protocol negotiation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

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
    pub operation_id: Uuid,
    pub deadline: u64,
}
impl Metadata {
    pub fn new(workspace_id: Option<Uuid>, timeout_ms: u64) -> Self {
        Self {
            protocol_version: VERSION,
            request_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            workspace_id,
            task_id: None,
            operation_id: Uuid::new_v4(),
            deadline: now_ms().saturating_add(timeout_ms),
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
pub enum Frame {
    Hello {
        protocol_version: u16,
        secret: Vec<u8>,
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
