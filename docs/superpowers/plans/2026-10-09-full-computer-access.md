# Plano de Implementação — Acesso ao Computador Inteiro (Full Computer Access)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implementar o modo Computador Inteiro no Remote Controller, permitindo ao ChatGPT ler e navegar em Downloads, Documentos, Desktop e todas as unidades do Windows a partir do perfil do usuário, com proteção estrita de credenciais locais (enclave) e consentimento explícito no instalador.

**Architecture:** O `WorkspaceFiles` ganha o modo `AccessScope::FullComputer` com raiz no perfil do usuário (`C:\Users\<User>`), suporte a caminhos relativos e absolutos de múltiplas unidades (`C:\`, `D:\`), e uma lista de bloqueio de enclave (`LOCALAPPDATA\RemoteDesktopController`, `.ssh`, `.aws`, credenciais, raízes do sistema para remoção). O `local-daemon` e `device-agent` ganham a flag `--full-computer`. O `install.ps1` adiciona o diálogo de consentimento explícito.

**Tech Stack:** Rust (cap-std, tokio, serde, cap-directories), PowerShell (Windows 5.1/7+).

**Spec:** `docs/superpowers/specs/2026-10-09-full-computer-access-design.md`

## Global Constraints
- Ponto de partida padrão no modo Computador Inteiro: diretório do usuário (`$env:USERPROFILE` / `C:\Users\<User>`).
- Suporte a caminhos absolutos com letras de unidade no Windows (`C:\...`, `D:\...`).
- Bloqueio inegociável de credenciais locais do agente (`LOCALAPPDATA\RemoteDesktopController`, `agent.json`, `ipc.secret`, `.rdc`).
- Bloqueio de chaves de usuário (`.ssh`, `.aws/credentials`, `.gnupg`).
- Bloqueio de exclusão (`filesystem.remove`) em raízes de unidade e pastas essenciais do sistema.
- Leitura ativada por padrão; escrita, processos e terminal desativados por padrão (exigem flags explícitas).
- Compatibilidade reversa integral para instalações no modo restrito.

---

### Task 1: Enclave Protection e Modo FullComputer em `crates/infrastructure/src/files.rs`

**Files:**
- Modify: `crates/infrastructure/src/files.rs`
- Test: `crates/infrastructure/tests/files_access.rs`

**Interfaces:**
- Produces: `AccessScope` (`Restricted`, `FullComputer`), `WorkspaceFiles::open_with_scope(root: &Path, limits: FileLimits, scope: AccessScope, enclaves: Vec<PathBuf>) -> Result<Self>`
- Consumes: `FileLimits`, `PortError`, `cap_std`

- [ ] **Step 1: Escrever teste falhando para resolução e enclave no modo FullComputer**
Criar `crates/infrastructure/tests/files_access.rs` testando:
1. Resolução de caminho relativo a partir do root (`Downloads/test.txt`).
2. Resolução de caminho absoluto (`<tempdir>/outside/file.txt`).
3. Rejeição com `PortError::Policy` ao tentar ler caminho do enclave (`.ssh/id_rsa`, `.rdc/secret`, `RemoteDesktopController/agent.json`).
4. Rejeição de remoção de raiz.

- [ ] **Step 2: Executar teste para verificar falha inicial**
Executar: `C:\Users\Notebook\.cargo\bin\cargo.exe test --test files_access`
Esperado: Falha de compilação/teste por tipos/métodos ausentes.

- [ ] **Step 3: Implementar `AccessScope` e validação de Enclave em `files.rs`**
1. Adicionar enum `AccessScope { Restricted, FullComputer }`.
2. Adicionar campo `scope: AccessScope` e `enclaves: Vec<PathBuf>` em `WorkspaceFiles`.
3. Implementar `resolve_path(&self, path: &str) -> Result<PathBuf>`:
   - Se `Restricted`: validação lexical clássica relativa ao `root`.
   - Se `FullComputer`:
     - Normalizar separadores `\` para `/`.
     - Validar ausência de null bytes `\0`, device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`), e ADS (`:` após letra da unidade).
     - Se relativo: combina com `self.root`.
     - Se absoluto: resolve no disco.
     - Checagem de Enclave: se o caminho canônico ou normalizado começar por qualquer caminho em `self.enclaves` ou contiver `.rdc` ou `.ssh`: retornar `Err(PortError::Policy("access to sensitive enclave denied".into()))`.
     - Para `remove`: se o caminho for raiz de volume (`C:\`, `D:\`) ou pasta raiz de usuário, rejeitar com `PortError::Policy`.
4. Atualizar métodos `read`, `read_range`, `stat`, `write`, `patch`, `remove`, `scan` para operar no caminho resolvido.

- [ ] **Step 4: Executar testes para confirmar aprovação**
Executar: `C:\Users\Notebook\.cargo\bin\cargo.exe test --test files_access`
Esperado: PASS.

- [ ] **Step 5: Commit**
`git add crates/infrastructure/src/files.rs crates/infrastructure/tests/files_access.rs`
`git commit -m "feat(infrastructure): implement full computer access scope and enclave protection"`

---

### Task 2: Suporte a `--full-computer` no `local-daemon`

**Files:**
- Modify: `apps/local-daemon/src/main.rs:80-140`
- Modify: `apps/local-daemon/src/lifecycle.rs:270-290`
- Modify: `crates/infrastructure/src/kernel.rs`

**Interfaces:**
- Produces: CLI arg `--full-computer` no daemon; repasse de `AccessScope::FullComputer` ao `Kernel::open`.

- [ ] **Step 1: Adicionar flag `--full-computer` aos argumentos do daemon**
Em `apps/local-daemon/src/main.rs`, adicionar campo `#[arg(long)] pub full_computer: bool` na struct `Args`.

- [ ] **Step 2: Ajustar checagem de `state_dir` e repassar enclave**
Em `main.rs`:
Quando `args.full_computer` estiver ativo:
- Não falhar se `state.starts_with(&root)`.
- Passar `state` e o diretório de dados do agente para a lista de enclaves do `Kernel`.
- Abrir `Kernel` com `AccessScope::FullComputer`.

- [ ] **Step 3: Testar compilação e testes do daemon**
Executar: `C:\Users\Notebook\.cargo\bin\cargo.exe test -p local-daemon`
Esperado: PASS.

- [ ] **Step 4: Commit**
`git add apps/local-daemon/src/main.rs apps/local-daemon/src/lifecycle.rs crates/infrastructure/src/kernel.rs`
`git commit -m "feat(daemon): add --full-computer flag and enclave registration"`

---

### Task 3: Suporte a `--full-computer` no `device-agent`

**Files:**
- Modify: `apps/device-agent/src/profile.rs`
- Modify: `apps/device-agent/src/main.rs`
- Modify: `apps/device-agent/src/daemon.rs`

**Interfaces:**
- Produces: `Profile.full_computer: bool`, flag `--full-computer` no comando `pair`, repasse de `--full-computer` ao iniciar o `local-daemon`.

- [ ] **Step 1: Atualizar struct `Profile` em `profile.rs`**
Adicionar `#[serde(default)] pub full_computer: bool`.
Em `protect_profile`: se `full_computer` for true, permitir que o perfil fique dentro do workspace (já que o workspace é a pasta do usuário), mantendo a proteção NTFS/ACL restrita do arquivo.

- [ ] **Step 2: Atualizar `main.rs` no comando `pair`**
Adicionar `#[arg(long)] full_computer: bool` ao `Action::Pair`.
Salvar `full_computer` no `Profile`.

- [ ] **Step 3: Atualizar `daemon.rs` para repassar `--full-computer`**
Em `daemon::add_args`:
Se `profile.full_computer`: adicionar `command.arg("--full-computer")`.

- [ ] **Step 4: Testar compilação e testes do `device-agent`**
Executar: `C:\Users\Notebook\.cargo\bin\cargo.exe test -p device-agent`
Esperado: PASS.

- [ ] **Step 5: Commit**
`git add apps/device-agent/src/profile.rs apps/device-agent/src/main.rs apps/device-agent/src/daemon.rs`
`git commit -m "feat(agent): support full_computer profile and daemon bootstrap"`

---

### Task 4: Fluxo de Consentimento no Instalador Windows (`install.ps1` e `setup-windows.ps1`)

**Files:**
- Modify: `tools/distribution/install.ps1:280-335`
- Modify: `tools/setup-windows.ps1:100-165`

**Interfaces:**
- Produces: Menu interativo claro para o usuário escolher entre [1] Computador Inteiro (Recomendado) e [2] Pasta Específica (Restrito).
- Switches: `-FullComputer` e `-Workspace` preservados.

- [ ] **Step 1: Atualizar `setup-windows.ps1`**
Adicionar parâmetro `[switch]$FullComputer`.
Se `$FullComputer` for fornecido:
- Permitir `$Workspace` como `$env:USERPROFILE`.
- Repassar `--full-computer` para `device-agent pair`.

- [ ] **Step 2: Atualizar `tools/distribution/install.ps1`**
No bloco de escolha do Workspace:
Apresentar opções numeradas claras:
```text
-- Permissão de Acesso aos Arquivos --
[1] Computador Inteiro (Recomendado):
    Permite à IA ler e navegar em pastas como Downloads, Documentos, Área de Trabalho e projetos no seu computador.
    As credenciais locais e arquivos sensíveis permanecem protegidos.
[2] Pasta Específica (Modo Restrito):
    Restringe a IA exclusivamente a uma pasta de sua escolha (ex: RemoteWorkspace).
```
Se 1:
`$Workspace = $env:USERPROFILE`, `$FullComputer = $true`, `$pairArgs += '--full-computer'`.
Se 2:
Solicitar caminho ou abrir FolderPicker.

- [ ] **Step 3: Testar script PowerShell localmente**
Executar validação de sintaxe e teste de parâmetros em PowerShell 5.1.

- [ ] **Step 4: Commit**
`git add tools/distribution/install.ps1 tools/setup-windows.ps1`
`git commit -m "feat(installer): add explicit consent prompt for full computer vs restricted workspace"`

---

### Task 5: Compilação de Binários, Pacote de Distribuição e Verificação Final

**Files:**
- Build: `target/release/device-agent.exe`, `target/release/local-daemon.exe`
- Update: `dist/remote-desktop-controller-windows-x64-0.3.0.zip` e manifestos
- Test: `tools/test-distribution-installer.ps1`

- [ ] **Step 1: Compilar binários Windows otimizados**
Executar: `C:\Users\Notebook\.cargo\bin\cargo.exe build --release -p device-agent -p local-daemon`
Verificar geração limpa dos binários em `target/release/`.

- [ ] **Step 2: Atualizar pacote de distribuição e calcular hashes SHA-256**
Atualizar arquivos em `dist/remote-desktop-controller-windows-x64-0.3.0/`.
Regerar zip e arquivo `.sha256`.

- [ ] **Step 3: Executar testes de distribuição**
Executar testes de instalação e validação.

- [ ] **Step 4: Commit e sincronização do repositório público de distribuição**
Atualizar `remotecontroller-windows` e registrar commits.
