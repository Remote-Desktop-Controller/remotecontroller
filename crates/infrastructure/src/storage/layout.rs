//! Directory layout and isolation manager for `~/.remotecontroller/`.
//!
//! Enforces physical isolation:
//! - App Binaries: `$env:LOCALAPPDATA\Programs\RemoteDesktopController\`
//! - State, Configuration, Checkpoints, and Logs: `$HOME\.remotecontroller\`
//! - Workspaces/Projects: External directories chosen by the user.
//!
//! Under no circumstances does uninstallation delete user workspaces or purge
//! `.remotecontroller` without an explicit `--purge-data` confirmation flag.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Structure representing the standard directory layout for Remote Desktop Controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLayout {
    root: PathBuf,
}

impl Default for StateLayout {
    fn default() -> Self {
        Self {
            root: Self::default_root(),
        }
    }
}

impl StateLayout {
    /// Resolves the default root directory for `.remotecontroller`.
    ///
    /// Precedence:
    /// 1. `REMOTECONTROLLER_HOME` environment variable (if set).
    /// 2. `RDC_STATE_DIR` environment variable (if set).
    /// 3. Windows `$USERPROFILE\.remotecontroller`.
    /// 4. Unix `$HOME/.remotecontroller`.
    /// 5. Current directory fallback `.remotecontroller`.
    pub fn default_root() -> PathBuf {
        if let Some(custom) = std::env::var_os("REMOTECONTROLLER_HOME") {
            return PathBuf::from(custom);
        }
        if let Some(custom) = std::env::var_os("RDC_STATE_DIR") {
            return PathBuf::from(custom);
        }

        #[cfg(windows)]
        {
            if let Some(profile) = std::env::var_os("USERPROFILE") {
                return PathBuf::from(profile).join(".remotecontroller");
            }
        }

        #[cfg(not(windows))]
        {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(".remotecontroller");
            }
        }

        // Fallback for isolated or non-standard environments
        PathBuf::from(".remotecontroller")
    }

    /// Constructs a layout rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns a reference to the root path of the layout.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the main configuration file: `config.json`.
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// Path to the sessions directory: `sessions/`.
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Path to a specific session state directory: `sessions/<session_id>/`.
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(session_id)
    }

    /// Path to the checkpoints root directory: `checkpoints/`.
    pub fn checkpoints_dir(&self) -> PathBuf {
        self.root.join("checkpoints")
    }

    /// Path to a specific session checkpoints directory: `checkpoints/<session_id>/`.
    pub fn session_checkpoints_dir(&self, session_id: &str) -> PathBuf {
        self.checkpoints_dir().join(session_id)
    }

    /// Path to the logs directory: `logs/`.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Path to a specific daily audit log file: `logs/audit-<date>.jsonl`.
    pub fn audit_log_path(&self, date: &str) -> PathBuf {
        self.logs_dir().join(format!("audit-{date}.jsonl"))
    }

    /// Path to today's daily audit log file based on UTC timestamp.
    pub fn daily_audit_log_path(&self) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Calculate YYYY-MM-DD from Unix seconds without external chrono dependency
        let days = now / 86400;
        let (year, month, day) = days_to_ymd(days);
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        self.audit_log_path(&date_str)
    }

    /// Path to the credentials vault directory: `.vault/`.
    pub fn vault_dir(&self) -> PathBuf {
        self.root.join(".vault")
    }

    /// Path to the DPAPI-encrypted secrets binary: `.vault/secrets.bin`.
    pub fn secrets_path(&self) -> PathBuf {
        self.vault_dir().join("secrets.bin")
    }

    /// Ensures all required layout directories exist with secure permissions.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.sessions_dir())?;
        std::fs::create_dir_all(self.checkpoints_dir())?;
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.vault_dir())?;

        // On Windows, restrict access if transport helper is available
        // Protect root and vault directory from other user accounts
        let _ = runtime_transport::private_directory(&self.root);
        let _ = runtime_transport::private_directory(&self.vault_dir());

        Ok(())
    }

    /// Safe purge operation that strictly requires affirmative confirmation.
    ///
    /// Refuses to execute if `confirm_purge_flag` is false, preventing accidental
    /// data deletion during standard uninstallations.
    pub fn purge_data(&self, confirm_purge_flag: bool) -> std::io::Result<()> {
        if !confirm_purge_flag {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Refusing to delete state directory without explicit confirm_purge_flag=true",
            ));
        }

        if self.root.exists() {
            std::fs::remove_dir_all(&self.root)?;
        }
        Ok(())
    }
}

/// Computes (year, month, day) from days elapsed since 1970-01-01 (Civil calendar algorithm).
fn days_to_ymd(days: u64) -> (u32, u32, u32) {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 14244) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_layout_subpaths() {
        let base = PathBuf::from("C:\\Users\\TestUser\\.remotecontroller");
        let layout = StateLayout::new(&base);

        assert_eq!(layout.root(), &base);
        assert_eq!(layout.config_path(), base.join("config.json"));
        assert_eq!(layout.sessions_dir(), base.join("sessions"));
        assert_eq!(
            layout.session_dir("sess-1"),
            base.join("sessions").join("sess-1")
        );
        assert_eq!(layout.checkpoints_dir(), base.join("checkpoints"));
        assert_eq!(
            layout.session_checkpoints_dir("sess-1"),
            base.join("checkpoints").join("sess-1")
        );
        assert_eq!(layout.logs_dir(), base.join("logs"));
        assert_eq!(
            layout.audit_log_path("2026-10-10"),
            base.join("logs").join("audit-2026-10-10.jsonl")
        );
        assert_eq!(layout.vault_dir(), base.join(".vault"));
        assert_eq!(
            layout.secrets_path(),
            base.join(".vault").join("secrets.bin")
        );
    }

    #[test]
    fn test_ensure_dirs_and_purge_protection() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".remotecontroller");
        let layout = StateLayout::new(&root);

        assert!(!root.exists());
        layout.ensure_dirs().unwrap();
        assert!(root.exists());
        assert!(layout.sessions_dir().exists());
        assert!(layout.checkpoints_dir().exists());
        assert!(layout.logs_dir().exists());
        assert!(layout.vault_dir().exists());

        // Attempting purge without confirmation flag fails with PermissionDenied
        let err = layout.purge_data(false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(root.exists());

        // With explicit confirmation, directory is purged
        layout.purge_data(true).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn test_daily_audit_log_format() {
        let layout = StateLayout::new("/tmp/.remotecontroller");
        let path = layout.daily_audit_log_path();
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.starts_with("audit-"));
        assert!(filename.ends_with(".jsonl"));
    }
}
