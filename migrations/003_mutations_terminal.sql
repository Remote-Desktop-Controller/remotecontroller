CREATE TABLE filesystem_journal(
    id TEXT PRIMARY KEY REFERENCES operations(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    payload BLOB NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('prepared','committed','rolled_back')),
    created_at INTEGER NOT NULL
);
CREATE INDEX filesystem_journal_workspace ON filesystem_journal(workspace_id,status);
CREATE TABLE terminals(
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    operation_id TEXT NOT NULL REFERENCES operations(id),
    status TEXT NOT NULL,
    exit_code INTEGER,
    created_at INTEGER NOT NULL
);
