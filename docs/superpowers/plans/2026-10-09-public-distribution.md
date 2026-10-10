# Public Distribution & Guided Windows Installation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide an automated, secure public distribution pipeline on GitHub Releases, a single-command Windows PowerShell installer with SHA-256 verification and guided pairing, and a visual onboarding experience in the portal dashboard including ChatGPT MCP setup.

**Architecture:** A GitHub Actions release workflow compiles Windows x64 binaries in release mode and packages them into verified release bundles with cryptographic SHA-256 manifests. A public installer script (`install.ps1`) downloads stable releases, verifies file hashes, installs cleanly to user local storage, guides folder selection and pairing, starts background execution with optional autostart, and provides a clean uninstaller. The portal dashboard guides users with a 1-click copyable command and visual MCP connection tutorial.

**Tech Stack:** Rust (Cargo workspace, `device-agent`, `local-daemon`), PowerShell (5.1+ & 7+), GitHub Actions, Next.js 16 / React 19, Tailwind CSS, Lucide icons, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-09-public-distribution-design.md`

## Global Constraints
- Target platform: Windows 10/11 x64.
- Official server URL: `https://andredozero.webajato.com.br`.
- Public distribution repo: `Remote-Desktop-Controller/remotecontroller-windows`.
- Strict SHA-256 integrity verification for all downloaded assets before execution.
- No elevation required (installs in `%LOCALAPPDATA%\Programs\RemoteDesktopController`).
- Read-only defaults: no write (`--allow-write`), no processes (`--allow-process`), no terminal (`--allow-terminal`) unless explicitly chosen by user.
- No publication to public repositories or VPS restarts without final user authorization.
- Preserve existing Premium customer capabilities and Free daily quota architecture.

---

### Task 1: GitHub Actions Release Workflow (`.github/workflows/release-windows.yml`)

**Files:**
- Create: `.github/workflows/release-windows.yml`
- Test: `tools/test-product-package.ps1`

**Interfaces:**
- Consumes: `tools/package-product.ps1`, `Cargo.toml`
- Produces: GitHub release workflow artifact containing `remote-desktop-controller-windows-x64-<version>.zip`, `manifest.json`, and SHA-256 sums.

- [ ] **Step 1: Write the workflow file**
Create `.github/workflows/release-windows.yml` configuring:
- Trigger on `workflow_dispatch` (with version input) and push tags (`v*`).
- Runner: `windows-latest`.
- Rust toolchain: 1.98.1.
- Release compilation of `device-agent` and `local-daemon`.
- Running package generation via `./tools/package-product.ps1`.
- Verifying archive hashes.
- Generating draft GitHub release or artifact upload ready for sync to public distribution repository.

- [ ] **Step 2: Validate packaging tool locally**
Run: `pwsh -File tools/package-product.ps1 -VerifyOnly`
Expected: Output valid JSON manifest with SHA-256 hashes for all binaries.

---

### Task 2: Public Distribution Repository Package & Scripts (`tools/distribution/`)

**Files:**
- Create: `tools/distribution/install.ps1`
- Create: `tools/distribution/uninstall.ps1`
- Create: `tools/distribution/README.md`
- Test: `tools/test-distribution-installer.ps1`

**Interfaces:**
- Consumes: GitHub Releases URL / local package path for testing.
- Produces: Self-contained installer script that downloads, validates SHA-256, configures, pairs, and manages lifecycle.

- [ ] **Step 1: Write `tools/distribution/install.ps1`**
Implement the user-facing bootstrap installer:
- Resolves latest stable release (or takes `-Version` parameter).
- Downloads release zip and `manifest.json` via HTTPS.
- Computes SHA-256 of downloaded zip and compares strictly against manifest hash.
- Unpacks to staging directory in `$env:LOCALAPPDATA\Programs\RemoteDesktopController\<version>`.
- Validates SHA-256 of `device-agent.exe` and `local-daemon.exe`.
- Interactive prompts (friendly for non-technical users):
  - Computer name: defaults to `$env:COMPUTERNAME`.
  - Workspace directory: opens FolderBrowserDialog (with fallback prompt, defaulting to `$HOME\RemoteWorkspace`).
- Configures server automatically: `https://andredozero.webajato.com.br`.
- Restrictive security: read-only access (no write, no unconfined processes, no terminal).
- Executes `device-agent.exe pair`.
- Automatically opens default browser at verification URL (`Start-Process $verificationUri`).
- Waits for user approval on portal.
- Prompts for autostart consent: `[S] Sim / [N] Não`. If approved, runs `device-agent autostart enable`.
- Starts `device-agent run` in background hidden window.
- Installs `uninstall.ps1` into program directory.
- Supports `-Update` mode (updates binaries, preserves profile).

- [ ] **Step 2: Write `tools/distribution/uninstall.ps1`**
Implement clean uninstallation:
- Stops running `device-agent` and associated daemon.
- Disables autostart Run entry in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- Asks whether to keep or remove user configuration profile in `%LOCALAPPDATA%\RemoteDesktopController`.
- Deletes installed binary folder `%LOCALAPPDATA%\Programs\RemoteDesktopController`.
- Outputs clean confirmation message.

- [ ] **Step 3: Write `tools/distribution/README.md`**
Documentation for public repository:
- One-line installation command.
- Requirements (Windows 10/11 x64).
- Security principles (SHA-256 integrity, local-first permissions, no admin rights required).
- Update and uninstall instructions.

---

### Task 3: Local Installation & Verification Test Harness

**Files:**
- Create: `tools/test-distribution-installer.ps1`
- Test: Run automated test of install, hash verification, tampering rejection, update, and uninstall.

- [ ] **Step 1: Create test script `tools/test-distribution-installer.ps1`**
Test cases:
1. Rejects corrupt/tampered zip file (SHA-256 mismatch detection).
2. Clean installation with simulated package into test destination.
3. Preserves existing configuration profile on re-run / update.
4. Clean uninstallation restores environment (removes files, stops background processes).

- [ ] **Step 2: Execute test harness**
Run: `powershell -ExecutionPolicy Bypass -File tools/test-distribution-installer.ps1`
Expected: All test cases pass with green output.

---

### Task 4: Portal Dashboard Onboarding & ChatGPT MCP Connection Guide

**Files:**
- Modify: `apps/portal/src/components/account-views.tsx`
- Modify: `apps/portal/src/components/portal.tsx`
- Create/Modify: `apps/portal/tests/installer-onboarding.test.tsx`

**Interfaces:**
- Consumes: Dashboard state (`account`), `useConfig`.
- Produces: Visual installation card with copy button, 3-step onboarding tracker, and ChatGPT MCP setup guide modal/section.

- [ ] **Step 1: Write test for installer onboarding components**
Create `apps/portal/tests/installer-onboarding.test.tsx` testing:
- Renders "Instalar no Windows" section.
- Renders the official PowerShell command string:
  `$p="$env:TEMP\remotecontroller-install.ps1"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p`
- Renders ChatGPT MCP connection tutorial details (`https://andredozero.webajato.com.br/mcp`).

- [ ] **Step 2: Update `apps/portal/src/components/account-views.tsx`**
Add:
- Highlighted Windows installer section in `Dashboard` view:
  - Button "Instalar no Windows" & Code block with Copy button.
  - Interactive copy feedback toast.
  - Step 1: Execute comando no PowerShell.
  - Step 2: Autorize o código no navegador.
  - Step 3: Conecte o MCP ao ChatGPT.
- Detailed visual guide modal/accordion for ChatGPT MCP connection:
  - Passos claros: Configurações -> Connected Apps -> Adicionar Servidor MCP.
  - URL do servidor: `https://andredozero.webajato.com.br/mcp`.
  - Explicação simples sobre a autorização de leitura e segurança.

- [ ] **Step 3: Run portal test suite**
Run: `npm test --prefix apps/portal`
Expected: All tests pass.

---

### Task 5: End-to-End Verification & Evidence Compilation

**Files:**
- Test: Full portal test suite (`vitest`).
- Test: Full cargo test suite for local-daemon and device-agent.
- Create: `docs/commercial-portal/public-distribution-validation.md`

- [ ] **Step 1: Run comprehensive tests across apps**
Execute:
- Rust tests: `cargo test -p local-daemon -p device-agent --locked`
- Portal tests: `npm test --prefix apps/portal`
- Distribution installer verification: `powershell -File tools/test-distribution-installer.ps1`

- [ ] **Step 2: Document verification evidence**
Write `docs/commercial-portal/public-distribution-validation.md` recording all test outputs, SHA-256 verification results, installer flow, and next steps for the user before publishing to GitHub.
