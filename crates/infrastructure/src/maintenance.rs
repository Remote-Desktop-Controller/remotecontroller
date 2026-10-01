//! Explicit local maintenance. The CLI must hold the daemon's exclusive lock:
//! SQLite snapshots are consistent, while filesystem spools require writer quiescence.
use crate::storage::{LocalStore, storage_error};
use runtime_ports::{PortError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Absolute timestamp cutoff. Only already rolled-back checkpoint records qualify.
    pub older_than_ms: u64,
    pub max_records: usize,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            older_than_ms: runtime_protocol::now_ms().saturating_sub(30 * 24 * 60 * 60 * 1000),
            max_records: 100,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupInfo {
    pub database_path: PathBuf,
    pub spool_directory: Option<PathBuf>,
    pub created_at_ms: u64,
    pub database_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintenanceReport {
    pub dry_run: bool,
    pub checkpoint_ids: Vec<String>,
    pub removed_records: u64,
    pub process_spool_ids: Vec<String>,
    pub removed_spools: u64,
    pub backup: Option<BackupInfo>,
    pub audit_path: Option<PathBuf>,
}
pub struct Maintenance {
    store: LocalStore,
}
const ELIGIBLE: &str = "c.status='rolled_back' AND c.updated_at < ?1 AND o.status IN ('Succeeded','Failed','Cancelled','TimedOut') AND NOT EXISTS (SELECT 1 FROM processes active WHERE active.operation_id=c.operation_id AND active.status IN ('Starting','Running','starting','running')) AND NOT EXISTS (SELECT 1 FROM checkpoints p WHERE p.operation_id=c.operation_id AND p.status='prepared')";
impl Maintenance {
    pub fn new(store: LocalStore) -> Self {
        Self { store }
    }
    pub async fn backup(&self, destination: &Path) -> Result<BackupInfo> {
        let destination = destination.to_owned();
        if destination.exists() {
            return Err(PortError::Policy(
                "backup destination already exists".into(),
            ));
        }
        let parent = destination.parent().unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err(PortError::Policy("backup parent must already exist".into()));
        }
        let source = self.store.state_dir().join("process-spool");
        if source.exists()
            && parent
                .canonicalize()
                .map_err(storage_error)?
                .starts_with(source.canonicalize().map_err(storage_error)?)
        {
            return Err(PortError::Policy(
                "backup destination cannot be inside the live spool".into(),
            ));
        }
        let path = destination
            .to_str()
            .ok_or_else(|| PortError::Policy("backup path must be UTF-8".into()))?
            .to_owned();
        // VACUUM INTO accepts an existing empty output. Reserve and protect it
        // before SQLite writes sensitive data outside the private state tree.
        drop(private_create(&destination)?);
        self.store
            .run(move |c| async move {
                c.execute("VACUUM INTO ?1", [path])
                    .await
                    .map_err(storage_error)?;
                Ok(())
            })
            .await?;
        let db = libsql::Builder::new_local(&destination)
            .build()
            .await
            .map_err(storage_error)?;
        let connection = db.connect().map_err(storage_error)?;
        let mut rows = connection
            .query("PRAGMA quick_check", ())
            .await
            .map_err(storage_error)?;
        let row = rows
            .next()
            .await
            .map_err(storage_error)?
            .ok_or_else(|| storage_error("backup quick_check returned no result"))?;
        if row.get::<String>(0).map_err(storage_error)? != "ok" {
            return Err(storage_error("backup quick_check failed"));
        }
        drop(rows);
        drop(connection);
        drop(db);
        let database_path = destination.clone();
        tokio::task::spawn_blocking(move || {
            let mut spool_directory = None;
            if source.exists() {
                let target = destination.with_extension("spool");
                if target.exists() {
                    return Err(PortError::Policy(
                        "backup spool destination already exists".into(),
                    ));
                }
                let mut bytes = 0_u64;
                let mut count = 0_usize;
                copy_private_tree(&source, &target, &mut bytes, &mut count)?;
                spool_directory = Some(target);
            }
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&database_path)
                .map_err(storage_error)?
                .sync_all()
                .map_err(storage_error)?;
            Ok(BackupInfo {
                database_bytes: fs::metadata(&database_path).map_err(storage_error)?.len(),
                database_path,
                spool_directory,
                created_at_ms: runtime_protocol::now_ms(),
            })
        })
        .await
        .map_err(storage_error)?
    }
    pub async fn plan(&self, policy: &RetentionPolicy) -> Result<MaintenanceReport> {
        if policy.max_records == 0 || policy.max_records > 10000 {
            return Err(PortError::Policy(
                "maintenance max_records must be 1..10000".into(),
            ));
        }
        let cutoff = policy.older_than_ms.min(i64::MAX as u64) as i64;
        let limit = policy.max_records as i64;
        let ids=self.store.run(move|c|async move {
            let mut rows=c.query(&format!("SELECT c.id FROM checkpoints c JOIN operations o ON o.id=c.operation_id WHERE {ELIGIBLE} ORDER BY c.updated_at,c.id LIMIT ?2"),libsql::params![cutoff,limit]).await.map_err(storage_error)?;
            let mut ids=Vec::new(); while let Some(row)=rows.next().await.map_err(storage_error)? {ids.push(row.get::<String>(0).map_err(storage_error)?);} Ok(ids)
        }).await?;
        let process_ids=self.store.run(move|c|async move {
            let mut rows=c.query("SELECT p.id FROM processes p JOIN operations o ON o.id=p.operation_id WHERE p.started_at < ?1 AND p.status NOT IN ('Starting','Running','starting','running') AND o.status IN ('Succeeded','Failed','Cancelled','TimedOut') AND NOT EXISTS (SELECT 1 FROM checkpoints c WHERE c.operation_id=p.operation_id AND c.status='prepared') ORDER BY p.started_at,p.id LIMIT ?2",libsql::params![cutoff,limit]).await.map_err(storage_error)?;
            let mut ids=Vec::new();while let Some(row)=rows.next().await.map_err(storage_error)? {ids.push(row.get::<String>(0).map_err(storage_error)?);}Ok(ids)
        }).await?;
        let mut process_spool_ids = Vec::new();
        for id in process_ids {
            let directory = spool_path(self.store.state_dir(), &id)?;
            if directory.exists() && validate_spool(&directory, policy.older_than_ms)? {
                process_spool_ids.push(id);
            }
        }
        Ok(MaintenanceReport {
            dry_run: true,
            checkpoint_ids: ids,
            removed_records: 0,
            process_spool_ids,
            removed_spools: 0,
            backup: None,
            audit_path: None,
        })
    }
    pub async fn run(
        &self,
        policy: &RetentionPolicy,
        dry_run: bool,
        backup_destination: Option<&Path>,
    ) -> Result<MaintenanceReport> {
        if dry_run {
            return self.plan(policy).await;
        }
        let destination = backup_destination.ok_or_else(|| {
            PortError::Policy("retention requires a new explicit backup destination".into())
        })?;
        let mut report = self.plan(policy).await?;
        let backup = self.backup(destination).await?;
        // The backup and intent audit are durable before any deletion.
        let audit_path = destination.with_extension("maintenance.json");
        report.dry_run = false;
        report.backup = Some(backup);
        report.audit_path = Some(audit_path.clone());
        let serialized = serde_json::to_vec_pretty(&report).map_err(storage_error)?;
        let mut audit = private_create(&audit_path)?;
        audit.write_all(&serialized).map_err(storage_error)?;
        audit.sync_all().map_err(storage_error)?;
        let ids = report.checkpoint_ids.clone();
        let cutoff = policy.older_than_ms.min(i64::MAX as u64) as i64;
        report.removed_records=self.store.run(move|c|async move {
            let tx=c.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await.map_err(storage_error)?;
            let mut removed=0;
            for id in ids {removed+=tx.execute(&format!("DELETE FROM checkpoints WHERE id=?2 AND id IN (SELECT c.id FROM checkpoints c JOIN operations o ON o.id=c.operation_id WHERE {ELIGIBLE})"),libsql::params![cutoff,id]).await.map_err(storage_error)?;}
            let mut check=tx.query("PRAGMA foreign_key_check",()).await.map_err(storage_error)?;
            if check.next().await.map_err(storage_error)?.is_some() {return Err(storage_error("maintenance foreign key check failed"));}
            drop(check);tx.commit().await.map_err(storage_error)?;Ok(removed)
        }).await?;
        // Caller holds the exclusive daemon fence. Refuse links and unknown
        // members; delete only the files owned by our spool format.
        for id in &report.process_spool_ids {
            let directory = spool_path(self.store.state_dir(), id)?;
            if !validate_spool(&directory, policy.older_than_ms)? {
                continue;
            }
            for name in ["stdout", "stderr", "metadata.json", "metadata.tmp"] {
                let file = directory.join(name);
                if file.exists() {
                    fs::remove_file(file).map_err(storage_error)?;
                }
            }
            fs::remove_dir(&directory).map_err(storage_error)?;
            report.removed_spools += 1;
        }
        Ok(report)
    }
}
fn spool_path(state: &Path, id: &str) -> Result<PathBuf> {
    id.parse::<runtime_domain::ProcessId>()
        .map_err(storage_error)?;
    let root = state.join("process-spool");
    if root.exists()
        && fs::symlink_metadata(&root)
            .map_err(storage_error)?
            .file_type()
            .is_symlink()
    {
        return Err(PortError::Policy("spool root cannot be a link".into()));
    }
    Ok(root.join(id))
}
fn validate_spool(directory: &Path, cutoff: u64) -> Result<bool> {
    let meta = fs::symlink_metadata(directory).map_err(storage_error)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(PortError::Policy("retention refuses spool links".into()));
    }
    for entry in fs::read_dir(directory).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        if !["stdout", "stderr", "metadata.json", "metadata.tmp"]
            .iter()
            .any(|name| entry.file_name() == *name)
        {
            return Err(PortError::Policy(
                "retention refuses unknown spool members".into(),
            ));
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(storage_error)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(PortError::Policy(
                "retention refuses special spool members".into(),
            ));
        }
        let modified = meta
            .modified()
            .map_err(storage_error)?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(storage_error)?
            .as_millis();
        if modified >= cutoff as u128 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn copy_private_tree(
    source: &Path,
    target: &Path,
    bytes: &mut u64,
    count: &mut usize,
) -> Result<()> {
    let meta = fs::symlink_metadata(source).map_err(storage_error)?;
    if meta.file_type().is_symlink() {
        return Err(PortError::Policy("backup refuses spool symlinks".into()));
    }
    if meta.is_dir() {
        runtime_transport::private_directory(target).map_err(storage_error)?;
        for entry in fs::read_dir(source).map_err(storage_error)? {
            let entry = entry.map_err(storage_error)?;
            copy_private_tree(&entry.path(), &target.join(entry.file_name()), bytes, count)?;
        }
    } else if meta.is_file() {
        *count += 1;
        *bytes = bytes.saturating_add(meta.len());
        // Four owned files per record including a crash-left metadata.tmp.
        // Align with the 4096-record cap and the maximum configured payload quota.
        if *count > 4096 * 4 || *bytes > 16 * 1024 * 1024 * 1024 + 16 * 1024 * 1024 {
            return Err(PortError::Policy(
                "backup spool exceeds safety bounds".into(),
            ));
        }
        let mut input = fs::File::open(source).map_err(storage_error)?;
        let mut output = private_create(target)?;
        std::io::copy(&mut input, &mut output).map_err(storage_error)?;
        output.sync_all().map_err(storage_error)?;
    } else {
        return Err(PortError::Policy(
            "backup refuses special spool files".into(),
        ));
    }
    Ok(())
}
fn private_create(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(storage_error)?;
    runtime_transport::private_file(path).map_err(storage_error)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_domain::*;
    use runtime_ports::{OperationRepository, WorkspaceRepository};
    #[cfg(target_os = "linux")]
    #[test]
    fn retention_directory_sync_order_is_durable() {
        let trace_dir = tempfile::tempdir().unwrap();
        let trace_path = trace_dir.path().join("syscalls.log");
        let output = std::process::Command::new("strace")
            .args([
                "-f",
                "-yy",
                "-e",
                "trace=fsync,fdatasync,unlink,unlinkat",
                "-o",
            ])
            .arg(&trace_path)
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "maintenance::tests::retention_backs_up_terminal_spool_and_preserves_running_spool",
                "--nocapture",
            ])
            .output()
            .expect("Linux durability gate requires strace");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let fixture = stdout
            .lines()
            .find_map(|line| line.strip_prefix("DURABILITY_FIXTURE|"))
            .unwrap();
        let fields: Vec<_> = fixture.split('|').collect();
        let root = Path::new(fields[0]);
        let trace = fs::read_to_string(trace_path).unwrap();
        let lines: Vec<_> = trace.lines().collect();
        let synced = |line: &&str, path: &Path| {
            line.contains("fsync(")
                && line.contains(&format!("<{}>", path.display()))
                && line.contains("= 0")
        };
        let audit = root.join("backup.maintenance.json");
        let audit_sync = lines
            .iter()
            .position(|line| synced(line, &audit))
            .expect("audit bytes must be synced");
        for directory in [
            root.join("backup.spool"),
            root.join("backup.spool").join(fields[1]),
            root.join("backup.spool").join(fields[2]),
        ] {
            let directory_sync = lines
                .iter()
                .position(|line| synced(line, &directory))
                .expect("backup directory entry must be synced before retention");
            assert!(
                directory_sync < audit_sync,
                "backup must be durable before its audit"
            );
        }
        let first_removal = lines
            .iter()
            .position(|line| {
                (line.contains("unlink(") || line.contains("unlinkat("))
                    && line.contains("/process-spool/")
            })
            .expect("fixture must remove a real live spool");
        assert!(
            lines[audit_sync + 1..first_removal]
                .iter()
                .any(|line| synced(line, root)),
            "backup and audit parent must be synced before removal"
        );
    }
    #[tokio::test]
    async fn retention_backs_up_terminal_spool_and_preserves_running_spool() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("state.db"))
            .await
            .unwrap();
        let ws = store.open_workspace("fixture").await.unwrap();
        let mut ids = Vec::new();
        for running in [false, true] {
            let op = Operation {
                id: OperationId::new(),
                workspace_id: ws.id,
                task_id: None,
                trace_id: TraceId::new(),
                tool: "process.spawn".into(),
                input: vec![],
                status: OperationStatus::Queued,
                result: None,
                created_at: 1,
            };
            store.claim(&op).await.unwrap();
            store.set_running(op.id).await.unwrap();
            if !running {
                store
                    .finish(
                        op.id,
                        OperationStatus::Succeeded,
                        &ToolResult {
                            output: vec![],
                            error: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            let id = ProcessId::new().to_string();
            let recorded = id.clone();
            store.run(move|c|async move {c.execute("INSERT INTO processes(id,workspace_id,operation_id,cwd,started_at,status) VALUES(?1,?2,?3,'.',1,?4)",libsql::params![recorded,ws.id.to_string(),op.id.to_string(),if running {"Running"} else {"Exited"}]).await.map_err(storage_error)?;Ok(())}).await.unwrap();
            let spool = dir.path().join("process-spool").join(&id);
            fs::create_dir_all(&spool).unwrap();
            for name in ["stdout", "stderr", "metadata.json", "metadata.tmp"] {
                fs::write(spool.join(name), b"preserved log").unwrap();
            }
            ids.push(id);
        }
        let m = Maintenance::new(store);
        let policy = RetentionPolicy {
            older_than_ms: u64::MAX,
            max_records: 10,
        };
        assert_eq!(
            m.plan(&policy).await.unwrap().process_spool_ids,
            vec![ids[0].clone()]
        );
        let backup = dir.path().join("backup.db");
        println!(
            "DURABILITY_FIXTURE|{}|{}|{}",
            dir.path().display(),
            ids[0],
            ids[1]
        );
        let report = m.run(&policy, false, Some(&backup)).await.unwrap();
        assert_eq!(report.removed_spools, 1);
        assert!(!dir.path().join("process-spool").join(&ids[0]).exists());
        assert!(dir.path().join("process-spool").join(&ids[1]).is_dir());
        assert_eq!(
            fs::read(backup.with_extension("spool").join(&ids[0]).join("stdout")).unwrap(),
            b"preserved log"
        );
    }
}
