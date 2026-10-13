//! Comprehensive integration tests for Phases 2, 3, and 4:
//! - State layout isolation & safe purge protection.
//! - DPAPI credential vault encryption and persistence.
//! - Atomic file writes, pre-mutation checkpoints, and rollback.
//! - Myers diff engine and binary file discriminator.
//! - Redactor regex sanitization pipeline.
//! - Observer IPC Named Pipe broadcasting and live event formatting.
//! - Kernel execution interception emitting real-time telemetry.

#![allow(unused_imports)]

use runtime_domain::ExecutionPolicy;
use runtime_infrastructure::{
    config::RuntimeConfig,
    kernel::Kernel,
    observer::{
        AuditDiffMetrics, AuditEvent, AuditEventType, AuditStatus, TelemetryHub, TelemetryServer,
    },
    storage::{
        atomic::AtomicCheckpointManager,
        layout::StateLayout,
        vault::{Vault, dpapi_decrypt, dpapi_encrypt},
    },
    telemetry::{
        diff::{MAX_DIFF_TEXT_SIZE, compute_content_diff, compute_file_diff, is_binary_payload},
        redactor::{Redactor, redact_value},
    },
};
use runtime_protocol::{Metadata, Request, RequestEnvelope};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_state_layout_contract_and_purge_protection() {
    let tmp = tempdir().unwrap();
    let state_root = tmp.path().join(".remotecontroller");
    let layout = StateLayout::new(&state_root);

    // Verify paths match the strict architecture specification
    assert_eq!(layout.root(), state_root.as_path());
    assert_eq!(layout.config_path(), state_root.join("config.json"));
    assert_eq!(layout.sessions_dir(), state_root.join("sessions"));
    assert_eq!(layout.checkpoints_dir(), state_root.join("checkpoints"));
    assert_eq!(layout.logs_dir(), state_root.join("logs"));
    assert_eq!(layout.vault_dir(), state_root.join(".vault"));
    assert_eq!(
        layout.secrets_path(),
        state_root.join(".vault").join("secrets.bin")
    );

    // Ensure directory creation
    layout.ensure_dirs().unwrap();
    assert!(state_root.exists());
    assert!(layout.sessions_dir().exists());
    assert!(layout.checkpoints_dir().exists());
    assert!(layout.logs_dir().exists());
    assert!(layout.vault_dir().exists());

    // Inviolable Rule: Accidental purge without confirmation flag is strictly forbidden
    let refusal = layout.purge_data(false);
    assert!(
        refusal.is_err(),
        "Purge without explicit confirmation must fail"
    );
    assert_eq!(
        refusal.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(state_root.exists(), "State directory must remain intact");

    // Intentional purge with explicit confirmation succeeds
    layout.purge_data(true).unwrap();
    assert!(
        !state_root.exists(),
        "State directory should be removed when confirmed"
    );
}

#[test]
fn test_dpapi_vault_encryption_integrity() {
    let tmp = tempdir().unwrap();
    let vault_file = tmp.path().join(".vault").join("secrets.bin");

    // 1. Raw DPAPI roundtrip
    let raw_secret = b"sk-proj-super-secret-api-token-999888";
    let ciphertext = dpapi_encrypt(raw_secret, None).unwrap();
    assert_ne!(ciphertext, raw_secret);

    let decrypted = dpapi_decrypt(&ciphertext, None).unwrap();
    assert_eq!(decrypted, raw_secret);

    // 2. High-level Vault storage
    {
        let mut vault = Vault::open(&vault_file).unwrap();
        vault
            .set("OPENAI_KEY", "sk-proj-openai-credential-secret")
            .unwrap();
        vault
            .set("ANTHROPIC_KEY", "sk-ant-anthropic-credential-secret")
            .unwrap();
        vault
            .set("SSH_PASSPHRASE", "super-secure-passphrase-value")
            .unwrap();

        assert_eq!(
            vault.get("OPENAI_KEY"),
            Some("sk-proj-openai-credential-secret")
        );
        assert_eq!(
            vault.get("ANTHROPIC_KEY"),
            Some("sk-ant-anthropic-credential-secret")
        );
        assert_eq!(
            vault.get("SSH_PASSPHRASE"),
            Some("super-secure-passphrase-value")
        );
        assert_eq!(
            vault.list_keys(),
            vec!["ANTHROPIC_KEY", "OPENAI_KEY", "SSH_PASSPHRASE"]
        );
    }

    // 3. Verify physical ciphertext on disk contains zero plaintext substrings
    let disk_bytes = std::fs::read(&vault_file).unwrap();
    let disk_str = String::from_utf8_lossy(&disk_bytes);
    assert!(!disk_str.contains("OPENAI_KEY"));
    assert!(!disk_str.contains("sk-proj-openai"));
    assert!(!disk_str.contains("ANTHROPIC_KEY"));
    assert!(!disk_str.contains("super-secure-passphrase"));

    // 4. Persistence across process re-instantiation
    {
        let mut vault = Vault::open(&vault_file).unwrap();
        assert_eq!(
            vault.get("OPENAI_KEY"),
            Some("sk-proj-openai-credential-secret")
        );
        assert_eq!(
            vault.get("ANTHROPIC_KEY"),
            Some("sk-ant-anthropic-credential-secret")
        );

        // Remove key
        assert!(vault.remove("OPENAI_KEY").unwrap());
        assert_eq!(vault.get("OPENAI_KEY"), None);
        assert_eq!(vault.list_keys(), vec!["ANTHROPIC_KEY", "SSH_PASSPHRASE"]);
    }

    // 5. Re-open to confirm deletion persisted
    {
        let vault = Vault::open(&vault_file).unwrap();
        assert_eq!(vault.get("OPENAI_KEY"), None);
        assert_eq!(
            vault.get("ANTHROPIC_KEY"),
            Some("sk-ant-anthropic-credential-secret")
        );
    }
}

#[test]
fn test_atomic_checkpoints_and_session_rollback() {
    let tmp = tempdir().unwrap();
    let state_root = tmp.path().join(".remotecontroller");
    let layout = StateLayout::new(&state_root);
    let manager = AtomicCheckpointManager::new(layout);

    let workspace_file1 = tmp.path().join("service.rs");
    let workspace_file2 = tmp.path().join("config.toml");

    let initial_service_content = b"pub fn run() -> i32 { 42 }\n";
    std::fs::write(&workspace_file1, initial_service_content).unwrap();

    let session_id = "test-session-rollback-101";

    // Mutation 1: Modify existing file
    let updated_service_content = b"pub fn run() -> i32 { 100 }\npub fn stop() {}\n";
    let record1 = manager
        .atomic_write_with_checkpoint(session_id, &workspace_file1, updated_service_content)
        .unwrap();

    assert!(!record1.was_created);
    assert_eq!(
        std::fs::read(&workspace_file1).unwrap(),
        updated_service_content
    );

    // Mutation 2: Create brand new file
    let new_config_content = b"[server]\nport = 8080\n";
    let record2 = manager
        .atomic_write_with_checkpoint(session_id, &workspace_file2, new_config_content)
        .unwrap();

    assert!(record2.was_created);
    assert!(workspace_file2.exists());
    assert_eq!(std::fs::read(&workspace_file2).unwrap(), new_config_content);

    // Verify session manifest contains both checkpoints
    let manifest = manager.load_session_manifest(session_id).unwrap();
    assert_eq!(manifest.len(), 2);
    assert_eq!(manifest[0].checkpoint_id, record1.checkpoint_id);
    assert_eq!(manifest[1].checkpoint_id, record2.checkpoint_id);

    // Rollback session
    let restored_count = manager.rollback_session(session_id).unwrap();
    assert_eq!(restored_count, 2);

    // Assert file 1 is restored byte-for-byte to its original pre-mutation state
    assert_eq!(
        std::fs::read(&workspace_file1).unwrap(),
        initial_service_content
    );

    // Assert file 2 was deleted since it did not exist before session
    assert!(!workspace_file2.exists());
}

#[test]
fn test_diff_engine_myers_and_binary_discrimination() {
    let tmp = tempdir().unwrap();
    let text_path = tmp.path().join("code.rs");

    let original = b"fn alpha() {\n    println!(\"one\");\n}\n";
    std::fs::write(&text_path, original).unwrap();

    // 1. Text diff calculation
    let modified = b"fn alpha() {\n    println!(\"one modified\");\n}\nfn beta() {}\n";
    let metrics = compute_file_diff(&text_path, modified);

    assert!(!metrics.is_binary);
    assert_eq!(metrics.lines_removed, 1);
    assert_eq!(metrics.lines_added, 2);
    assert_eq!(metrics.total_bytes, modified.len());

    // 2. Binary detection via null bytes
    let binary_with_null = b"ELF\0\x01\x02\x03compiled_executable";
    assert!(is_binary_payload(binary_with_null));

    let bin_metrics = compute_content_diff(original, binary_with_null);
    assert!(bin_metrics.is_binary);
    assert_eq!(bin_metrics.lines_added, 0);
    assert_eq!(bin_metrics.lines_removed, 0);
    assert_eq!(bin_metrics.total_bytes, binary_with_null.len());

    // 3. Binary detection via oversized payload (> 2MB)
    let oversized = vec![b'x'; MAX_DIFF_TEXT_SIZE + 64];
    assert!(is_binary_payload(&oversized));
    let oversized_metrics = compute_content_diff(b"", &oversized);
    assert!(oversized_metrics.is_binary);
    assert_eq!(oversized_metrics.total_bytes, oversized.len());
}

#[test]
fn test_secret_redactor_comprehensive() {
    let redactor = Redactor::new();

    // OpenAI key
    let raw_openai = "Connecting with key sk-proj-1234567890abcdef1234567890 and fallback sk-11223344556677889900aabbcc";
    let cleaned_openai = redactor.redact(raw_openai);
    assert!(!cleaned_openai.contains("sk-proj-"));
    assert!(!cleaned_openai.contains("sk-1122"));
    assert!(cleaned_openai.contains("[REDACTED_OPENAI_KEY]"));

    // Anthropic key
    let raw_anthropic = "ANTHROPIC_API_KEY=sk-ant-api03-abcdef1234567890abcdef1234567890";
    let cleaned_anthropic = redactor.redact(raw_anthropic);
    assert!(!cleaned_anthropic.contains("sk-ant-"));
    assert!(cleaned_anthropic.contains("[REDACTED_ANTHROPIC_KEY]"));

    // Bearer token
    let raw_bearer = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.token1234567890";
    let cleaned_bearer = redactor.redact(raw_bearer);
    assert!(!cleaned_bearer.contains("eyJhbGciOi"));
    assert!(cleaned_bearer.contains("Bearer [REDACTED_TOKEN]"));

    // SSH / PKCS#8 private keys (both RSA-specific and generic PKCS#8 format)
    let raw_rsa = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0abcdef1234567890\n-----END RSA PRIVATE KEY-----";
    let cleaned_rsa = redactor.redact(raw_rsa);
    assert!(!cleaned_rsa.contains("MIIEowIBAAK"));
    assert!(cleaned_rsa.contains("[REDACTED_PRIVATE_KEY]"));

    let raw_pkcs8 = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQD\n-----END PRIVATE KEY-----";
    let cleaned_pkcs8 = redactor.redact(raw_pkcs8);
    assert!(!cleaned_pkcs8.contains("MIIEvgIBADAN"));
    assert!(cleaned_pkcs8.contains("[REDACTED_PRIVATE_KEY]"));

    // Bearer tokens (including Base64 +, /, = characters)
    let raw_bearer = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.token1234567890";
    let cleaned_bearer = redactor.redact(raw_bearer);
    assert!(!cleaned_bearer.contains("eyJhbGciOi"));
    assert!(cleaned_bearer.contains("Bearer [REDACTED_TOKEN]"));

    let raw_bearer_base64 = "Authorization: Bearer dGVzdCthYmMvMTIzNDU2Nzg5MDEyMzQ1Njc4OTA=";
    let cleaned_bearer_b64 = redactor.redact(raw_bearer_base64);
    assert!(!cleaned_bearer_b64.contains("dGVzdCthYmM"));
    assert!(cleaned_bearer_b64.contains("Bearer [REDACTED_TOKEN]"));

    // GitHub & GitLab tokens (including fine-grained github_pat_)
    let raw_git = "ghp_123456789012345678901234567890123456, github_pat_11A6W3D4Q0abc123def456ghi789jkl012, glpat-abcdef1234567890abcdef";
    let cleaned_git = redactor.redact(raw_git);
    assert!(!cleaned_git.contains("ghp_"));
    assert!(!cleaned_git.contains("github_pat_"));
    assert!(!cleaned_git.contains("glpat-"));
    assert!(cleaned_git.contains("[REDACTED_GITHUB_TOKEN]"));
    assert!(cleaned_git.contains("[REDACTED_GITLAB_TOKEN]"));

    // JSON tree sanitization (verifying password and token values are redacted)
    let json_input = json!({
        "auth": {
            "token": "sk-proj-99887766554433221100",
            "password": "supersecretpassword123"
        },
        "tags": ["safe", "sk-ant-api03-999888777666555444333222"],
        "count": 10
    });

    let cleaned_json = redact_value(&json_input);
    let json_str = cleaned_json.to_string();
    assert!(!json_str.contains("sk-proj-"));
    assert!(!json_str.contains("sk-ant-"));
    assert!(
        !json_str.contains("supersecretpassword123"),
        "Secret password MUST be redacted"
    );
    assert_eq!(cleaned_json["count"], 10);
}

#[cfg(windows)]
#[tokio::test]
async fn test_observer_ipc_streaming_and_interception() {
    let pipe_name = format!(r"\\.\pipe\rdc-test-suite-{}", uuid::Uuid::new_v4().simple());
    let hub = TelemetryHub::new(64);
    let server = TelemetryServer::new(hub.clone(), &pipe_name);
    let cancel = server.cancel_token();
    let _server_task = server.start();

    // Initialize pipe
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect observer client
    let mut rx = runtime_infrastructure::observer::server::connect_telemetry_client(&pipe_name)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // 1. Broadcast FILE_MODIFIED
    let file_event =
        AuditEvent::file_modified("crates/domain/src/lib.rs", 15, 3, 45, AuditStatus::Success);
    hub.broadcast(file_event);

    let rec1 = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timeout on file_event")
        .expect("channel closed");
    assert_eq!(rec1.event_type, AuditEventType::FileModified);
    assert_eq!(rec1.target, "crates/domain/src/lib.rs");
    assert_eq!(
        rec1.diff,
        Some(AuditDiffMetrics {
            added: 15,
            removed: 3
        })
    );
    assert_eq!(rec1.status, AuditStatus::Success);

    // 2. Broadcast COMMAND_EXECUTED with secret to verify on-the-fly redaction
    let cmd_event = AuditEvent::command_executed(
        "git push https://x-access-token:ghp_123456789012345678901234567890123456@github.com/repo.git",
        120,
        AuditStatus::Success,
        None,
    );
    hub.broadcast(cmd_event);

    let rec2 = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timeout on cmd_event")
        .expect("channel closed");
    assert_eq!(rec2.event_type, AuditEventType::CommandExecuted);
    assert!(!rec2.target.contains("ghp_1234"));
    assert!(rec2.target.contains("[REDACTED_GITHUB_TOKEN]"));

    // 3. Display formatting check
    let display_str = rec1.format_display(true);
    assert!(display_str.contains("FILE_MODIFIED"));
    assert!(display_str.contains("+15"));
    assert!(display_str.contains("-3"));

    let plain_display_str = rec1.format_display(false);
    assert!(plain_display_str.contains("[+15/-3]"));

    cancel.cancel();
}

#[tokio::test]
async fn test_kernel_execution_emits_telemetry() {
    use runtime_domain::Capability;

    let root = tempdir().unwrap();
    let state = tempdir().unwrap();

    let kernel = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]),
    )
    .await
    .unwrap();

    let mut telemetry_rx = kernel.telemetry.subscribe();

    // 1. Execute filesystem.write through kernel
    let target_file = "generated.txt";
    let payload = json!({
        "path": target_file,
        "content": "line 1\nline 2\n"
    });

    let meta = Metadata::new(Some(kernel.workspace.id.as_uuid()), 10_000);
    let request = RequestEnvelope {
        meta: meta.clone(),
        payload: Request::Execute {
            tool: "filesystem.write".into(),
            input: payload,
        },
    };

    let (progress_tx, _) = tokio::sync::mpsc::channel(16);
    let progress = Arc::new(runtime_infrastructure::kernel::ChannelProgress {
        sender: progress_tx,
        meta: meta.clone(),
    });

    let response = kernel.handle(request, progress).await;
    assert!(response.payload.is_ok(), "{:?}", response.payload);

    // Verify telemetry event was emitted with Myers diff
    let audit_event = tokio::time::timeout(Duration::from_secs(2), telemetry_rx.recv())
        .await
        .expect("timeout waiting for telemetry event")
        .expect("telemetry channel error");

    assert_eq!(audit_event.event_type, AuditEventType::FileModified);
    assert_eq!(audit_event.target, target_file);
    assert_eq!(audit_event.status, AuditStatus::Success);
    assert_eq!(
        audit_event.diff,
        Some(AuditDiffMetrics {
            added: 2,
            removed: 0
        })
    );

    // 2. Execute filesystem.patch through kernel and verify real-time Myers diff
    let patch_payload = json!({
        "path": target_file,
        "find": "line 2\n",
        "replace": "line 2 modified\nline 3 appended\n"
    });
    let patch_meta = Metadata::new(Some(kernel.workspace.id.as_uuid()), 10_000);
    let patch_request = RequestEnvelope {
        meta: patch_meta.clone(),
        payload: Request::Execute {
            tool: "filesystem.patch".into(),
            input: patch_payload,
        },
    };
    let (p_tx, _) = tokio::sync::mpsc::channel(16);
    let p_progress = Arc::new(runtime_infrastructure::kernel::ChannelProgress {
        sender: p_tx,
        meta: patch_meta,
    });
    let patch_response = kernel.handle(patch_request, p_progress).await;
    assert!(
        patch_response.payload.is_ok(),
        "{:?}",
        patch_response.payload
    );

    let patch_event = tokio::time::timeout(Duration::from_secs(2), telemetry_rx.recv())
        .await
        .expect("timeout waiting for patch telemetry")
        .expect("telemetry channel error");

    assert_eq!(patch_event.event_type, AuditEventType::FileModified);
    assert_eq!(patch_event.target, target_file);
    assert_eq!(patch_event.status, AuditStatus::Success);
    assert_eq!(
        patch_event.diff,
        Some(AuditDiffMetrics {
            added: 2,
            removed: 1
        })
    );

    // 3. Verify audit log was persisted to disk under ~/.remotecontroller/logs/audit-<date>.jsonl
    tokio::time::sleep(Duration::from_millis(100)).await;
    let audit_log_file = kernel.layout.daily_audit_log_path();
    assert!(
        audit_log_file.exists(),
        "Daily audit log file must exist at {}",
        audit_log_file.display()
    );
    let log_contents = std::fs::read_to_string(&audit_log_file).unwrap();
    assert!(log_contents.contains(target_file));
    assert!(log_contents.contains("FILE_MODIFIED"));
}
