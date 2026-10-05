CREATE TABLE sessions(
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    label TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('open','closed')) DEFAULT 'open',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(id, workspace_id)
);
ALTER TABLE operations ADD COLUMN session_id TEXT REFERENCES sessions(id);
CREATE INDEX sessions_workspace_seq ON sessions(workspace_id,seq DESC);
CREATE INDEX operations_session ON operations(workspace_id,session_id);
-- Defense in depth: SQL clients cannot bypass the workspace binding.
CREATE TRIGGER operations_session_scope_insert BEFORE INSERT ON operations
WHEN NEW.session_id IS NOT NULL AND NOT EXISTS(
    SELECT 1 FROM sessions WHERE id=NEW.session_id AND workspace_id=NEW.workspace_id
)
BEGIN SELECT RAISE(ABORT,'session workspace mismatch'); END;
CREATE TRIGGER operations_session_scope_update BEFORE UPDATE OF session_id,workspace_id ON operations
WHEN NEW.session_id IS NOT NULL AND NOT EXISTS(
    SELECT 1 FROM sessions WHERE id=NEW.session_id AND workspace_id=NEW.workspace_id
)
BEGIN SELECT RAISE(ABORT,'session workspace mismatch'); END;
