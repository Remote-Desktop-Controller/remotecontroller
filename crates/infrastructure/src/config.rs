use crate::{files::FileLimits, processes::ProgramRule};
use runtime_application::SchedulerLimits;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub scheduler: SchedulerConfig,
    pub files: FileLimits,
    pub allowed_programs: Vec<ProgramRule>,
    pub process_buffer_bytes: usize,
    pub process_spool_bytes: u64,
    pub process_spool_total_bytes: u64,
    #[serde(skip)]
    pub guardian_path: Option<std::path::PathBuf>,
    pub cache_capacity: u64,
    pub cache_ttl_seconds: u64,
    pub operation_timeout_ms: u64,
    pub ipc_connections: usize,
    pub progress_capacity: usize,
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
            files: FileLimits::default(),
            allowed_programs: vec![],
            process_buffer_bytes: 64 * 1024,
            process_spool_bytes: 16 * 1024 * 1024,
            process_spool_total_bytes: 256 * 1024 * 1024,
            guardian_path: None,
            cache_capacity: 256,
            cache_ttl_seconds: 10_800,
            operation_timeout_ms: 60_000,
            ipc_connections: 32,
            progress_capacity: 32,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerConfig {
    pub queue: usize,
    pub file_reads: usize,
    pub file_writes: usize,
    pub processes: usize,
    pub database: usize,
    pub cpu: usize,
    pub background: usize,
}
impl Default for SchedulerConfig {
    fn default() -> Self {
        let l = SchedulerLimits::default();
        Self {
            queue: l.queue,
            file_reads: l.file_reads,
            file_writes: l.file_writes,
            processes: l.processes,
            database: l.database,
            cpu: l.cpu,
            background: l.background,
        }
    }
}
impl From<SchedulerConfig> for SchedulerLimits {
    fn from(l: SchedulerConfig) -> Self {
        Self {
            queue: l.queue,
            file_reads: l.file_reads,
            file_writes: l.file_writes,
            processes: l.processes,
            database: l.database,
            cpu: l.cpu,
            background: l.background,
        }
    }
}
impl RuntimeConfig {
    pub fn validate(&self) -> runtime_ports::Result<()> {
        if self.ipc_connections == 0
            || self.process_spool_bytes == 0
            || self.process_spool_bytes > 1024 * 1024 * 1024
            || self.process_spool_total_bytes < self.process_spool_bytes
            || self.process_spool_total_bytes > 16 * 1024 * 1024 * 1024
            || self.ipc_connections > 1024
            || self.progress_capacity == 0
            || self.progress_capacity > 1024
            || self.operation_timeout_ms == 0
            || self.operation_timeout_ms > 3_600_000
            || self.files.max_file_bytes == 0
            || self.files.max_file_bytes > 2 * 1024 * 1024
            || self.files.max_batch_bytes == 0
            || self.files.max_batch_bytes > 128 * 1024 * 1024
            || self.files.max_batch_files == 0
            || self.files.max_batch_files > 100_000
            || self.files.max_scan_entries == 0
            || self.files.max_scan_entries > 1_000_000
            || self.cache_capacity == 0
            || self.cache_ttl_seconds == 0
        {
            return Err(runtime_ports::PortError::Policy(
                "invalid runtime limits".into(),
            ));
        }
        Ok(())
    }
}
