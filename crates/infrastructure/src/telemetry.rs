//! Telemetry subsystem: Diff computation and data sanitization.

pub mod diff;
pub mod redactor;

pub use diff::{DiffMetrics, compute_content_diff, compute_file_diff, is_binary_payload};
pub use redactor::{Redactor, redact_text, redact_value};
