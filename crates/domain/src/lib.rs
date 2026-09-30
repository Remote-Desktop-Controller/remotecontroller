//! Pure domain: no serialization, runtime, storage or operating-system dependencies.

use std::{collections::BTreeSet, fmt, str::FromStr};
use uuid::Uuid;

macro_rules! ids {
    ($($name:ident),+) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);
        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
            pub fn as_uuid(self) -> Uuid { self.0 }
        }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl FromStr for $name {
            type Err = DomainError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self).map_err(|_| DomainError::InvalidId)
            }
        }
    )+};
}
ids!(
    WorkspaceId,
    TaskId,
    OperationId,
    ExecutionSessionId,
    ToolCallId,
    EventId,
    ArtifactId,
    CheckpointId,
    ProcessId,
    TraceId
);

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid identifier")]
    InvalidId,
    #[error("invalid state transition")]
    InvalidTransition,
    #[error("capability denied: {0:?}")]
    PermissionDenied(Capability),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}
impl OperationStatus {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
    pub fn transition(self, next: Self) -> Result<Self, DomainError> {
        if (self == Self::Queued
            && matches!(
                next,
                Self::Running | Self::Failed | Self::Cancelled | Self::TimedOut
            ))
            || (self == Self::Running && next.terminal())
        {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    ReadWorkspace,
    WriteWorkspace,
    SpawnProcess,
    KillProcess,
    GitRead,
    GitWrite,
    NetworkAccess,
    PrivilegedShell,
    OutsideWorkspaceAccess,
}
#[derive(Clone, Debug)]
pub struct Permission {
    pub capability: Capability,
    pub granted_by: String,
}
#[derive(Clone, Debug)]
pub struct ExecutionPolicy {
    granted: BTreeSet<Capability>,
}
impl ExecutionPolicy {
    pub fn read_only() -> Self {
        Self::new([Capability::ReadWorkspace, Capability::GitRead])
    }
    pub fn new(granted: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }
    pub fn authorize(&self, cap: Capability) -> Result<(), DomainError> {
        self.granted
            .contains(&cap)
            .then_some(())
            .ok_or(DomainError::PermissionDenied(cap))
    }
    pub fn capabilities(&self) -> impl Iterator<Item = Capability> + '_ {
        self.granted.iter().copied()
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub root: String,
    pub graph_version: u64,
}
#[derive(Clone, Debug)]
pub struct Task {
    pub id: TaskId,
    pub workspace_id: WorkspaceId,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct ExecutionSession {
    pub id: ExecutionSessionId,
    pub workspace_id: WorkspaceId,
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub id: OperationId,
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub trace_id: TraceId,
    pub tool: String,
    pub input: Vec<u8>,
    pub status: OperationStatus,
    pub result: Option<ToolResult>,
    pub created_at: u64,
}
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub operation_id: OperationId,
    pub tool: String,
}
#[derive(Clone, Debug)]
pub struct ToolResult {
    pub output: Vec<u8>,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub id: EventId,
    pub workspace_id: WorkspaceId,
    pub operation_id: OperationId,
    pub kind: String,
    pub summary: String,
    pub raw: Vec<u8>,
    pub created_at: u64,
}
#[derive(Clone, Debug)]
pub struct EventNode {
    pub id: EventId,
    pub operation_id: OperationId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    CausedBy,
    DependsOn,
    FollowedBy,
    Mutated,
    Produced,
    RolledBackFrom,
}
#[derive(Clone, Debug)]
pub struct EventEdge {
    pub from: EventId,
    pub to: EventId,
    pub kind: EdgeKind,
}
#[derive(Clone, Debug)]
pub struct Artifact {
    pub id: ArtifactId,
    pub operation_id: OperationId,
    pub path: String,
}
#[derive(Clone, Debug)]
pub struct FileSnapshot {
    pub path: String,
    pub content: Option<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub workspace_id: WorkspaceId,
    pub operation_id: OperationId,
    pub files: Vec<FileSnapshot>,
    pub committed: bool,
}
#[derive(Clone, Debug)]
pub struct ProcessHandle {
    pub id: ProcessId,
    pub operation_id: OperationId,
    pub workspace_id: WorkspaceId,
    pub os_pid: Option<u32>,
    pub exit_code: Option<i32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Workload {
    FileRead,
    FileWrite,
    Process,
    Database,
    Cpu,
    Background,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_operations_cannot_restart() {
        assert!(
            OperationStatus::Queued
                .transition(OperationStatus::Running)
                .is_ok()
        );
        assert!(
            OperationStatus::Running
                .transition(OperationStatus::Cancelled)
                .is_ok()
        );
        assert!(
            OperationStatus::Succeeded
                .transition(OperationStatus::Running)
                .is_err()
        );
    }
    #[test]
    fn policy_never_grants_unlisted_capability() {
        let p = ExecutionPolicy::read_only();
        assert!(p.authorize(Capability::ReadWorkspace).is_ok());
        assert!(p.authorize(Capability::SpawnProcess).is_err());
        assert!(p.authorize(Capability::OutsideWorkspaceAccess).is_err());
    }
}
