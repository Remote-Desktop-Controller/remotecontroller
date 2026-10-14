# Project Conventions & Architecture Guardrails

## RDC Maestro & Daemon Core Guardrails

1. **Tauri Toolchain on Windows GNU (MinGW)**:
   - In `apps/rdc-maestro/src-tauri/Cargo.toml`, keep `crate-type = ["rlib"]` exclusively.
   - Do NOT re-add `cdylib` or `staticlib` targets on the Windows GNU toolchain to avoid `ld: export ordinal too large` and `.rsrc` merge failures during linking.

2. **Search/Replace Surgical Patch Invariant**:
   - Any surgical patch implementation (`<<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE` or `patch_search_replace`) must reject empty or blank search blocks immediately with `SearchBlockNotFound` before attempting substring checks or disk mutations.
   - Rust's `str.contains("")` evaluates to `true`; explicit whitespace/empty checks are mandatory.

3. **Rollback Receipts**:
   - All checkpoint restoration acks (`rdc.maestro.ack.v1`) must include the `restored_files: Vec<String>` list populated from the restored checkpoint's files metadata (`cp.files`).

4. **Compliance Gate (`context_log`)**:
   - When a `context_log` is supplied in `rdc.maestro.order.v1`, it must be logged in structured daemon logs and attached to the `AuditEvent` published on `TelemetryHub`.

5. **Physical Execution Invariant**:
   - All physical file, process, and checkpoint mutations must pass strictly through `Kernel::dispatch` to preserve path sandboxing, `--allow-write` security policies, and automatic checkpointing.

6. **Webview Preload Stability (`preload.js`)**:
   - Preserve ChatGPT idle debounce stabilization (`idleMinChars = 20`, `idleSettleMs = 2000`).
   - Preserve ProseMirror targeting for Claude (`div[contenteditable="true"].ProseMirror`).
   - Maintain top-level window guard (`window === window.top`) to prevent execution in sub-frames.
