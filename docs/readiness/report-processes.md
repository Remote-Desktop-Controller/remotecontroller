# Process readiness handoff

Implemented in `processes.rs`, `processes/process_spool.rs`, Windows `platform.rs`
and process integration tests. Existing constructor retained.

- `ProgramRule` adds `fingerprint: Option<String>` and `trusted_unconfined: bool`
  (serde defaults deny); `ProgramRule::fingerprint(&Path)` computes streaming
  SHA-256 for explicit local approval. Approved hash checked at construction and
  again immediately before spawn. Exact canonical executable/argv required;
  shells still denied. Result declares `execution_profile=trusted_unconfined`.
  Existing application process.spawn metadata requires current outside-workspace
  and network capabilities. No approval MCP tool added; root owns operator CLI.
- `ProcessManager::with_guardian(PathBuf) -> Result<Self>`: Unix fails closed
  without helper; launches `[--process-guardian, approved_program, exact_args...]`
  with validated cwd/env and piped stdin retained as heartbeat. Cancellation
  closes heartbeat and waits helper to clean target descendants. Root implements
  helper in composition root. No unsafe fork in Tokio.
- Windows target starts `CREATE_SUSPENDED|CREATE_NO_WINDOW`, is assigned to a
  kill-on-close Job, then its exact-one initial thread is resumed. Assignment or
  resume failures kill/wait the target before returning.
- `with_spool_limits(per_process_bytes, total_bytes) -> Result<Self>`; defaults
  16 MiB combined stdout/stderr per process, 256 MiB total payload. Private
  `process-spool/<ProcessId>/{stdout,stderr,metadata.json}`. Unix 0700 directories,
  0600 files; Windows inherits state-directory ACL. Append+flush+sync_data precede
  RAM updates, metadata uses synced temporary file and atomic rename (plus Unix
  directory sync). Budget reconstructs stored payload on restart. Metadata and
  inodes are bounded by 4096 record cap; explicit retention is needed afterward.
- Repository row validates workspace before spool reads. Status after eviction
  or restart includes bytes seen/stored, bounded output tail, truncation,
  capture_incomplete, raw refs, profile and completed duration_ms. Quota exhaustion
  continues draining and reports truncation. Crash gap between synced data and
  metadata preserves bytes and marks accounting incomplete. Interrupted processes
  report incomplete capture even when already committed bytes are readable.
- Recovery reconciles both Starting and Running to interrupted, never signals a
  persisted PID. Capture drain/finish failures are recorded as CaptureFailed.
  No SQL migration required for spool metadata; existing legacy BLOB rows remain
  readable and disclose missing spool capture metadata.

Evidence: test-first focused `operation_cancellation_kills_child_and_records_cancelled`
failed specifically for absent stdout_ref before changes (real Windows child).
Focused GREEN attempt was blocked by concurrent shared FileSnapshot/ports edits;
root centralizes final build/test gates. Added real spool quota/reopen and
data-sync crash-gap tests, approved-executable replacement denial, legacy-rule
deny defaults. Ownership files formatted with Rustfmt. No broad build or commit.
Added OS-backed descendant-tree cancellation fixture: approved test executable
spawns a sleeping descendant, test reads its real PID through persisted stdout,
cancels operation, and checks the descendant exited. Execution pending root gates.

Remaining proof: parent must build helper then run cancellation/tree/crash tests
on Unix CI and Windows gates; native macOS/Linux evidence cannot be claimed from
this Windows session. Root must secure Windows state ACL, wire current_exe helper,
operator approval and quotas, and include process-spool in retention/backups.
Pinning is a pre-execution content check, not protection against same-user malware
or a replacement race after that check. No OS sandbox claimed. Payload quotas
exclude filesystem allocation overhead and bounded metadata. Abrupt crash may
lose pipe bytes not yet read/synced; confirmed spool bytes remain recoverable.
