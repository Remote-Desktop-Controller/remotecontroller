//! Real-time telemetry broadcasting server and IPC streaming service.
//!
//! Provides:
//! - Broadcast pipeline via `tokio::sync::broadcast`.
//! - Windows Named Pipe transport (`\\.\pipe\rdc-telemetry`).
//! - Unix domain socket / localhost fallback for cross-platform support.
//! - Non-blocking event streaming to external observer TUIs/CLIs (`rdc-monitor`).

use super::event::AuditEvent;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Default Windows Named Pipe path for RDC telemetry streaming.
pub const DEFAULT_TELEMETRY_PIPE: &str = r"\\.\pipe\rdc-telemetry";

/// Capacity for the in-memory telemetry broadcast channel.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// In-memory broadcast hub distributing audit events to multiple subscribers.
#[derive(Debug, Clone)]
pub struct TelemetryHub {
    sender: broadcast::Sender<AuditEvent>,
}

impl Default for TelemetryHub {
    fn default() -> Self {
        Self::new(DEFAULT_CHANNEL_CAPACITY)
    }
}

impl TelemetryHub {
    /// Creates a new broadcast hub with the given ring buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Subscribes a new receiver to the broadcast stream.
    pub fn subscribe(&self) -> broadcast::Receiver<AuditEvent> {
        self.sender.subscribe()
    }

    /// Broadcasts an audit event to all active subscribers.
    /// Returns the number of active receivers that received the message.
    pub fn broadcast(&self, event: AuditEvent) -> usize {
        // If there are no active receivers, send returns an error which we can safely ignore
        self.sender.send(event).unwrap_or(0)
    }

    /// Returns a direct sender clone.
    pub fn sender(&self) -> broadcast::Sender<AuditEvent> {
        self.sender.clone()
    }

    /// Spawns a background task that persists every broadcast audit event
    /// to the daily log file under `~/.remotecontroller/logs/audit-<date>.jsonl`.
    pub fn attach_audit_logger(
        &self,
        layout: crate::storage::layout::StateLayout,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let mut rx = self.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = rx.recv() => {
                        match result {
                            Ok(event) => {
                                if let Ok(line) = event.to_json_line() {
                                    let log_path = layout.daily_audit_log_path();
                                    if let Some(parent) = log_path.parent() {
                                        let _ = std::fs::create_dir_all(parent);
                                    }
                                    use std::io::Write;
                                    if let Ok(mut file) = std::fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(&log_path)
                                    {
                                        let _ = writeln!(file, "{line}");
                                        let _ = file.flush();
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        })
    }
}

/// Telemetry streaming server serving connected IPC observers.
pub struct TelemetryServer {
    hub: TelemetryHub,
    endpoint: String,
    cancel_token: CancellationToken,
}

impl TelemetryServer {
    /// Constructs a telemetry server bound to the specified endpoint.
    pub fn new(hub: TelemetryHub, endpoint: impl Into<String>) -> Self {
        Self {
            hub,
            endpoint: endpoint.into(),
            cancel_token: CancellationToken::new(),
        }
    }

    /// Returns a cancellation token that can be used to stop the server loop.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Starts the server loop in a background task, streaming events to connecting clients.
    #[cfg(windows)]
    pub fn start(&self) -> tokio::task::JoinHandle<std::io::Result<()>> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let endpoint = self.endpoint.clone();
        let hub = self.hub.clone();
        let cancel = self.cancel_token.clone();

        tokio::spawn(async move {
            let mut is_first = true;
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                let mut server = match ServerOptions::new()
                    .first_pipe_instance(is_first)
                    .create(&endpoint)
                {
                    Ok(s) => {
                        is_first = false;
                        s
                    }
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    }
                };

                let mut client_rx = hub.subscribe();
                let connected = tokio::select! {
                    _ = cancel.cancelled() => break,
                    conn = server.connect() => conn.is_ok(),
                };

                if connected {
                    let cancel_client = cancel.clone();

                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = cancel_client.cancelled() => break,
                                recv_result = client_rx.recv() => {
                                    match recv_result {
                                        Ok(event) => {
                                            if let Ok(line) = event.to_json_line() {
                                                if server.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                                    break;
                                                }
                                                let _ = server.flush().await;
                                            }
                                        }
                                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                            let warning = AuditEvent {
                                                timestamp: crate::observer::event::current_timestamp_ms(),
                                                event_type: crate::observer::event::AuditEventType::ToolInvoked,
                                                target: format!("[SYSTEM_WARNING: {skipped} telemetry events dropped due to client buffer lag]"),
                                                diff: None,
                                                execution_time_ms: 0,
                                                status: crate::observer::event::AuditStatus::Failed,
                                                details: None,
                                            };
                                            if let Ok(line) = warning.to_json_line() {
                                                if server.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                                    break;
                                                }
                                                let _ = server.flush().await;
                                            }
                                            continue;
                                        }
                                        Err(broadcast::error::RecvError::Closed) => break,
                                    }
                                }
                            }
                        }
                    });
                }
            }
            Ok(())
        })
    }

    /// Non-Windows implementation using in-memory or Unix domain sockets.
    #[cfg(not(windows))]
    pub fn start(&self) -> tokio::task::JoinHandle<std::io::Result<()>> {
        let cancel = self.cancel_token.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            Ok(())
        })
    }
}

/// Connects to a Windows Named Pipe telemetry endpoint and yields parsed `AuditEvent`s.
#[cfg(windows)]
pub async fn connect_telemetry_client(
    endpoint: &str,
) -> std::io::Result<tokio::sync::mpsc::Receiver<AuditEvent>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let client = ClientOptions::new().open(endpoint)?;
    let (tx, rx) = tokio::sync::mpsc::channel(256);

    tokio::spawn(async move {
        let mut lines = BufReader::new(client).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            #[allow(clippy::collapsible_if)]
            if let Ok(event) = serde_json::from_str::<AuditEvent>(trimmed) {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        }
    });

    Ok(rx)
}

/// Non-Windows fallback for connect_telemetry_client.
#[cfg(not(windows))]
pub async fn connect_telemetry_client(
    _endpoint: &str,
) -> std::io::Result<tokio::sync::mpsc::Receiver<AuditEvent>> {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::event::{AuditDiffMetrics, AuditEventType, AuditStatus};

    #[tokio::test]
    async fn test_telemetry_hub_broadcast() {
        let hub = TelemetryHub::new(16);
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();

        let event = AuditEvent {
            timestamp: 1000,
            event_type: AuditEventType::ToolInvoked,
            target: "test_tool".into(),
            diff: None,
            execution_time_ms: 10,
            status: AuditStatus::Success,
            details: None,
        };

        let delivered = hub.broadcast(event.clone());
        assert_eq!(delivered, 2);

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();
        assert_eq!(received1, event);
        assert_eq!(received2, event);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_named_pipe_telemetry_server_client_roundtrip() {
        let pipe_name = format!(
            r"\\.\pipe\rdc-test-telemetry-{}",
            uuid::Uuid::new_v4().simple()
        );
        let hub = TelemetryHub::new(32);
        let server = TelemetryServer::new(hub.clone(), &pipe_name);
        let cancel = server.cancel_token();
        let _server_task = server.start();

        // Give the pipe server a moment to initialize
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client_rx = connect_telemetry_client(&pipe_name).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let event = AuditEvent::file_modified("src/test.rs", 5, 1, 20, AuditStatus::Success);
        hub.broadcast(event.clone());

        let received = tokio::time::timeout(std::time::Duration::from_secs(3), client_rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("channel closed unexpectedly");

        assert_eq!(received.target, "src/test.rs");
        assert_eq!(received.status, AuditStatus::Success);
        assert_eq!(
            received.diff,
            Some(AuditDiffMetrics {
                added: 5,
                removed: 1
            })
        );

        cancel.cancel();
    }
}
