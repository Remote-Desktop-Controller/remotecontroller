# Public Repository Release v0.5.0: Architecture RFCs, Technical Documentation & Remote Synchronization

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish public-ready RFCs, architectural blueprints, and developer documentation, bump the project workspace to v0.5.0, and push clean commits, tags, and branches to the now-public GitHub repository without modifying existing runtime logic.

**Architecture:** Remote Desktop Controller (RDC) is a multi-tier, local-first system consisting of: (1) Local Daemon with libSQL event graph & hardened named pipe IPC, (2) MCP Gateway translating stdio to authenticated IPC envelopes, (3) RDC Maestro Desktop orchestrating web LLMs (ChatGPT, Claude, Gemini, DeepSeek, Qwen) with stealth biomechanical CDP drivers, (4) Distributed Device Agent & Remote Service with mutual handshake v2, and (5) Commercial SaaS Portal with Asaas billing and public distribution packaging.

**Tech Stack:** Rust 1.88+ (Tokio, libSQL, rmcp, Windows API, DPAPI), TypeScript, React 19, Tailwind CSS v4, Tauri v2, PowerShell 5.1+, GitHub Actions / Git.

**Spec:** User request: "tornei o repositório do Git público, crie as documentações e rfcs e envie, atualize também o código lá, com a nova versão também, lá tem a tag 0.2.0, acredito que estamos na 0.5.0. não vamos mudar as coisas, vamos só atualizar, porque privada tava sem mexer".

## Global Constraints

- Do not alter or break working application logic or database schemas ("não vamos mudar as coisas, vamos só atualizar").
- Bump workspace package version from `0.2.0` to `0.5.0` across workspace manifests.
- Produce production-grade, highly structured RFCs and documentation matching elite open-source standards.
- Synchronize all commits, branch states, and the `v0.5.0` release tag with `origin` (`https://github.com/Remote-Desktop-Controller/remotecontroller.git`).

---

### Task 1: Verify Workspace Build and Clean Compilation

**Files:**
- Check: `Cargo.toml`
- Check: `apps/local-daemon/src/main.rs`
- Check: `apps/local-daemon/src/lifecycle.rs`
- Check: `apps/rdc-maestro/src-tauri/src/main.rs`

- [ ] **Step 1: Wait for and confirm `cargo check --workspace` clean exit**
- [ ] **Step 2: Run test suite on local daemon args and maestro channel**
- [ ] **Step 3: Verify git status to ensure working tree contains no unintended artifacts**

### Task 2: Version Synchronization to v0.5.0

**Files:**
- Modify: `Cargo.toml:10`
- Modify: `apps/rdc-maestro/package.json:4`
- Modify: `apps/rdc-maestro/src-tauri/tauri.conf.json:4`

- [ ] **Step 1: Update workspace version in root `Cargo.toml` to `0.5.0`**
- [ ] **Step 2: Update `apps/rdc-maestro/package.json` version to `0.5.0`**
- [ ] **Step 3: Update `apps/rdc-maestro/src-tauri/tauri.conf.json` version to `0.5.0`**
- [ ] **Step 4: Run `cargo check --workspace` to update Cargo.lock version references**

### Task 3: Author RFC Suite (RFC-0002 to RFC-0005 + RFC Index)

**Files:**
- Create: `docs/rfcs/README.md`
- Create: `docs/rfcs/0002-remote-pairing-protocol.md`
- Create: `docs/rfcs/0003-full-computer-access-enclave.md`
- Create: `docs/rfcs/0004-rdc-maestro-stealth-orchestrator.md`
- Create: `docs/rfcs/0005-commercial-portal-distribution.md`

- [ ] **Step 1: Write `docs/rfcs/README.md` cataloging RFC-0001 through RFC-0005 with architecture evolution**
- [ ] **Step 2: Write `docs/rfcs/0002-remote-pairing-protocol.md` (Mutual IPC v2, Pairing Codes, Session State, Remote Proxying)**
- [ ] **Step 3: Write `docs/rfcs/0003-full-computer-access-enclave.md` (Full Computer Control, Protected Enclave, DPAPI Vault, Observer Telemetry)**
- [ ] **Step 4: Write `docs/rfcs/0004-rdc-maestro-stealth-orchestrator.md` (RDC Maestro Tauri v2 Desktop, Multi-LLM Blackboard, Bézier Trajectories, Anti-Bot Sentinels)**
- [ ] **Step 5: Write `docs/rfcs/0005-commercial-portal-distribution.md` (SaaS Portal Architecture, Supabase & Asaas Integration, Multi-Platform v0.5.0 Bundler)**

### Task 4: Public Repository Documentation & Architectural Blueprint

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/security.md`

- [ ] **Step 1: Overhaul `README.md` for public presentation (Bilingual / Portuguese + English, System Architecture, Quickstart, Tool Catalog, RFC links, v0.5.0 Status)**
- [ ] **Step 2: Update `docs/architecture.md` to map the v0.5.0 unified system architecture with Mermaid diagrams**
- [ ] **Step 3: Update `docs/security.md` detailing the security model for Full Computer Access, Enclave Isolation, DPAPI and IPC tokens**

### Task 5: Git Commit, Tagging v0.5.0, and Remote Push

**Files:**
- Git repository state across branches `feat/maestro-stealth-architecture`, `main`, and tag `v0.5.0`.

- [ ] **Step 1: Stage and commit the implementation and version bump on `feat/maestro-stealth-architecture`**
- [ ] **Step 2: Commit all newly authored RFCs and documentation**
- [ ] **Step 3: Fast-forward / integrate the v0.5.0 release into `main`**
- [ ] **Step 4: Create annotated Git tag `v0.5.0`**
- [ ] **Step 5: Push branches `main`, `feat/maestro-stealth-architecture`, and tag `v0.5.0` to `origin`**
- [ ] **Step 6: Verify remote status with `git ls-remote origin`**
