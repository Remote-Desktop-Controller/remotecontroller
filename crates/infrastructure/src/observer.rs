//! Real-time telemetry observer and IPC streaming subsystem.

pub mod event;
pub mod server;

pub use event::{AuditDiffMetrics, AuditEvent, AuditEventType, AuditStatus};
pub use server::{DEFAULT_TELEMETRY_PIPE, TelemetryHub, TelemetryServer};
