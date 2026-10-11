//! Atomic file modification and checkpoint/rollback manager.
//!
//! Enforces:
//! - Checkpoint creation before any file mutation in user workspaces.
//! - Atomic writes via `tempfile::NamedTempFile` in the target's parent directory.
//! - Full rollback capability restoring original bytes or removing newly created files.

use super::layout::StateLayout;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AtomicStorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Checkpoint not found: {0}")]
    CheckpointNotFound(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

pub type Result<T> = std::result::Result<T, AtomicStorageError>;

/// Record of a file mutation checkpoint for audit and rollback.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CheckpointRecord {
    pub checkpoint_id: String,
    pub session_id: String,
    pub target_path: PathBuf,
    pub timestamp_ms: u64,
    pub was_created: bool,
    pub original_sha256: Option<String>,
    pub original_size_bytes: Option<u64>,
    pub backup_file_rel: Option<String>,
}

/// Manager responsible for creating checkpoints and safely writing workspace files atomically.
#[derive(Debug, Clone)]
pub struct AtomicCheckpointManager {
    layout: StateLayout,
}

impl AtomicCheckpointManager {
    /// Creates a manager configured with the specified state layout.
    pub fn new(layout: StateLayout) -> Self {
        Self { layout }
    }

    /// Atomically writes `content` to `target_path` within `session_id`,
    /// automatically taking a pre-mutation backup checkpoint.
    pub fn atomic_write_with_checkpoint(
        &self,
        session_id: &str,
        target_path: &Path,
        content: &[u8],
    ) -> Result<CheckpointRecord> {
        let canonical_target = if target_path.exists() {
            target_path.canonicalize().unwrap_or_else(|_| target_path.to_path_buf())
        } else {
            target_path.to_path_buf()
        };

        let session_checkpoint_dir = self.layout.session_checkpoints_dir(session_id);
        std::fs::create_dir_all(&session_checkpoint_dir)?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let checkpoint_id = format!("{now_ms}-{}", uuid::Uuid::new_v4().simple());

        let record = if canonical_target.exists() {
            let original_bytes = std::fs::read(&canonical_target)?;
            let mut hasher = Sha256::new();
            hasher.update(&original_bytes);
            let original_sha256 = format!("{:x}", hasher.finalize());

            let backup_filename = format!("{checkpoint_id}.bak");
            let backup_path = session_checkpoint_dir.join(&backup_filename);
            std::fs::write(&backup_path, &original_bytes)?;

            CheckpointRecord {
                checkpoint_id: checkpoint_id.clone(),
                session_id: session_id.to_string(),
                target_path: canonical_target.clone(),
                timestamp_ms: now_ms,
                was_created: false,
                original_sha256: Some(original_sha256),
                original_size_bytes: Some(original_bytes.len() as u64),
                backup_file_rel: Some(backup_filename),
            }
        } else {
            CheckpointRecord {
                checkpoint_id: checkpoint_id.clone(),
                session_id: session_id.to_string(),
                target_path: canonical_target.clone(),
                timestamp_ms: now_ms,
                was_created: true,
                original_sha256: None,
                original_size_bytes: None,
                backup_file_rel: None,
            }
        };

        // Write content atomically using tempfile in parent directory
        if let Some(parent) = canonical_target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let parent_dir = canonical_target.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent_dir)?;
        use std::io::Write;
        temp.write_all(content)?;
        temp.flush()?;
        temp.persist(&canonical_target).map_err(|e| e.error)?;

        // Append checkpoint record to session manifest
        self.append_manifest_record(session_id, &record)?;

        Ok(record)
    }

    /// Restores a single checkpoint, reversing the mutation.
    pub fn rollback_checkpoint(&self, record: &CheckpointRecord) -> Result<()> {
        if record.was_created {
            if record.target_path.exists() {
                std::fs::remove_file(&record.target_path)?;
            }
        } else if let Some(ref backup_rel) = record.backup_file_rel {
            let session_dir = self.layout.session_checkpoints_dir(&record.session_id);
            let backup_path = session_dir.join(backup_rel);
            if !backup_path.exists() {
                return Err(AtomicStorageError::CheckpointNotFound(format!(
                    "Backup file not found at {}",
                    backup_path.display()
                )));
            }

            let original_bytes = std::fs::read(&backup_path)?;

            // Atomically replace target with original bytes
            let parent_dir = record
                .target_path
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let mut temp = tempfile::NamedTempFile::new_in(parent_dir)?;
            use std::io::Write;
            temp.write_all(&original_bytes)?;
            temp.flush()?;
            temp.persist(&record.target_path).map_err(|e| e.error)?;
        }

        Ok(())
    }

    /// Performs a full rollback of all operations recorded in `session_id`,
    /// in reverse chronological order. Returns the count of rolled-back operations.
    pub fn rollback_session(&self, session_id: &str) -> Result<usize> {
        let manifest = self.load_session_manifest(session_id)?;
        let mut count = 0;

        for record in manifest.iter().rev() {
            self.rollback_checkpoint(record)?;
            count += 1;
        }

        Ok(count)
    }

    /// Appends a checkpoint entry to `checkpoints/<session_id>/manifest.jsonl`.
    fn append_manifest_record(&self, session_id: &str, record: &CheckpointRecord) -> Result<()> {
        let session_dir = self.layout.session_checkpoints_dir(session_id);
        std::fs::create_dir_all(&session_dir)?;
        let manifest_path = session_dir.join("manifest.jsonl");

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&manifest_path)?;

        let line = serde_json::to_string(record)?;
        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(())
    }

    /// Loads all checkpoint records for the specified session from `manifest.jsonl`.
    pub fn load_session_manifest(&self, session_id: &str) -> Result<Vec<CheckpointRecord>> {
        let session_dir = self.layout.session_checkpoints_dir(session_id);
        let manifest_path = session_dir.join("manifest.jsonl");

        if !manifest_path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&manifest_path)?;
        let mut records = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let rec: CheckpointRecord = serde_json::from_str(trimmed)?;
                records.push(rec);
            }
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_write_new_file_and_rollback() {
        let tmp = tempdir().unwrap();
        let state_dir = tmp.path().join(".remotecontroller");
        let layout = StateLayout::new(&state_dir);
        let manager = AtomicCheckpointManager::new(layout);

        let workspace_file = tmp.path().join("created_by_agent.rs");
        let session_id = "test-session-create";

        // 1. Write new file
        let content = b"fn main() { println!(\"hello\"); }";
        let record = manager
            .atomic_write_with_checkpoint(session_id, &workspace_file, content)
            .unwrap();

        assert!(record.was_created);
        assert!(workspace_file.exists());
        assert_eq!(std::fs::read(&workspace_file).unwrap(), content);

        // 2. Rollback newly created file
        manager.rollback_checkpoint(&record).unwrap();
        assert!(!workspace_file.exists(), "Rollback of created file must delete it");
    }

    #[test]
    fn test_atomic_write_overwrite_and_rollback() {
        let tmp = tempdir().unwrap();
        let state_dir = tmp.path().join(".remotecontroller");
        let layout = StateLayout::new(&state_dir);
        let manager = AtomicCheckpointManager::new(layout);

        let workspace_file = tmp.path().join("existing_code.rs");
        let initial_content = b"pub fn legacy() -> bool { true }";
        std::fs::write(&workspace_file, initial_content).unwrap();

        let session_id = "test-session-modify";
        let new_content = b"pub fn updated() -> bool { false }";

        // 1. Overwrite with checkpoint
        let record = manager
            .atomic_write_with_checkpoint(session_id, &workspace_file, new_content)
            .unwrap();

        assert!(!record.was_created);
        assert_eq!(std::fs::read(&workspace_file).unwrap(), new_content);
        assert!(record.backup_file_rel.is_some());

        // 2. Rollback
        manager.rollback_checkpoint(&record).unwrap();
        assert_eq!(
            std::fs::read(&workspace_file).unwrap(),
            initial_content,
            "Rollback must restore original bytes exactly"
        );
    }

    #[test]
    fn test_session_batch_rollback() {
        let tmp = tempdir().unwrap();
        let state_dir = tmp.path().join(".remotecontroller");
        let layout = StateLayout::new(&state_dir);
        let manager = AtomicCheckpointManager::new(layout);

        let file1 = tmp.path().join("file1.txt");
        let file2 = tmp.path().join("file2.txt");

        std::fs::write(&file1, b"file 1 original").unwrap();
        // file2 does not exist initially

        let session_id = "batch-session-1";
        manager
            .atomic_write_with_checkpoint(session_id, &file1, b"file 1 modified")
            .unwrap();
        manager
            .atomic_write_with_checkpoint(session_id, &file2, b"file 2 created")
            .unwrap();

        assert_eq!(std::fs::read(&file1).unwrap(), b"file 1 modified");
        assert!(file2.exists());

        // Full session rollback
        let reverted_count = manager.rollback_session(session_id).unwrap();
        assert_eq!(reverted_count, 2);

        assert_eq!(std::fs::read(&file1).unwrap(), b"file 1 original");
        assert!(!file2.exists());
    }
}
