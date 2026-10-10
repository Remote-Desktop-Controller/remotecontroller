# Especificação de Design — Acesso ao Computador Inteiro (Full Computer Access)

- **Data:** 2026-10-09
- **Status:** Proposto / Em Validação
- **Autor:** Antigravity & Engenharia Remote Controller
- **Alvo:** `remotecontroller-windows` (Instalador, Device-Agent, Local-Daemon e Crates de Infraestrutura)

---

## 1. Visão Geral e Motivação

O objetivo central do Remote Controller é permitir que assistentes de IA (como o ChatGPT via MCP) auxiliem o usuário em seu fluxo real de trabalho — lendo documentos, analisando relatórios baixados em Downloads, navegando em projetos e automatizando tarefas cotidianas.

Atualmente, o instalador e o daemon confinam todas as operações de arquivos a uma pasta única isolada (`C:\Users\<User>\RemoteWorkspace`). Qualquer tentativa de ler `Downloads`, `Documentos`, `Área de Trabalho` ou arquivos em partições secundárias (`D:\`) é imediatamente bloqueada com erro de segurança (`absolute/namespace path denied` ou `path traversal denied`).

Esta especificação define a arquitetura para suportar o **Modo Computador Inteiro** com consentimento explícito durante a instalação, mantendo a opção de **Modo Restrito** e garantindo isolamento absoluto de credenciais e segurança do sistema.

---

## 2. Requisitos e Diretrizes

### 2.1 Funcionais
1. **Consentimento Explícito na Instalação:**
   - Durante a execução do `install.ps1`, o usuário escolhe entre:
     - `[1] Computador Inteiro (Recomendado)`: Ponto de partida no perfil do usuário (`C:\Users\<User>`), com permissão de navegação por caminhos relativos e absolutos em todas as unidades (`C:\`, `D:\`, etc.).
     - `[2] Pasta Específica (Modo Restrito)`: Confinamento a uma pasta isolada (ex: `RemoteWorkspace`).
2. **Navegação Natural no Computador Inteiro:**
   - Suporte a caminhos relativos a partir do perfil do usuário (`Downloads/relatorio.pdf`, `Desktop/notas.txt`).
   - Suporte a caminhos absolutos no Windows (`C:\Projetos\app`, `D:\Documentos\contrato.docx`, `C:\Users\Notebook\Downloads`).
   - Operações em subpastas e discos existentes respeitando permissões do Windows do usuário logado.
3. **Controles Independentes de Risco:**
   - Leitura: Ativada por padrão.
   - Escrita: Desativada por padrão (exige flag/consentimento `--allow-write`).
   - Terminal e Processos: Desativados por padrão (exigem `--allow-process` e `--allow-terminal`).
4. **Compatibilidade:**
   - Instalações existentes mantêm suas configurações e modo restrito inalterados.
   - O protocolo MCP e os schemas das ferramentas (`filesystem.read`, `filesystem.write`, `filesystem.list`, `filesystem.stat`, etc.) permanecem retrocompatíveis.

### 2.2 Segurança e Proteção (Enclave Protegido)
Mesmo no modo Computador Inteiro:
1. **Credenciais do Remote Controller (Enclave Local):**
   - Bloqueio estrito de acesso a `$env:LOCALAPPDATA\RemoteDesktopController` (onde ficam `agent.json`, `ipc.secret`, logs de bootstrap).
   - Bloqueio de arquivos e diretórios iniciados por `.rdc` ou `.RDC-`.
2. **Credenciais do Usuário:**
   - Bloqueio estrito de leitura/escrita em diretórios de chaves privadas: `.ssh` (ex: `id_rsa`, `id_ed25519`), `.aws/credentials`, `.gnupg`.
3. **Proteção contra Exclusões Destrutivas (`filesystem.remove`):**
   - Proibição de exclusão de raízes de disco (`C:\`, `D:\`).
   - Proibição de exclusão dos diretórios essenciais do perfil do usuário (`C:\Users\<User>`, `Desktop`, `Documents`, `Downloads`).
   - Proibição de exclusão de diretórios do sistema operacional (`C:\Windows`, `C:\Program Files`, `C:\ProgramData`).
4. **Proteção de Dispositivos e Nomes Reservados:**
   - Manutenção do bloqueio de nomes reservados do DOS/Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`).
   - Manutenção do bloqueio de Alternate Data Streams (ADS) maliciosos e null bytes (`\0`).

---

## 3. Arquitetura e Componentes Modificados

### 3.1 `crates/infrastructure/src/files.rs` (`WorkspaceFiles`)
- **Novo Modo de Acesso:**
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub enum AccessScope {
      Restricted,
      FullComputer,
  }
  ```
- **Resolução de Caminhos (`resolve_path`):**
  - No modo `Restricted`: Mantém a regra atual (confinamento estrito a `self.root` via `Dir::open_ambient_dir`).
  - No modo `FullComputer`:
    - Valida ausência de null bytes e nomes de dispositivo reservados.
    - Se o caminho for relativo: combina com `self.root` (perfil do usuário).
    - Se o caminho for absoluto no Windows (ex: `C:\...` ou `D:\...`): normaliza e valida contra a lista de caminhos protegidos (Enclave Blacklist).
    - Aplica a checagem de Enclave: se o caminho canônico ou normalizado residir dentro de `LOCALAPPDATA\RemoteDesktopController`, `.ssh`, `.aws`, etc., retorna `PortError::Policy("access to sensitive enclave denied")`.
- **Operações Multidisco:**
  - `read`, `read_range`, `stat`, `write`, `patch`, `remove`, `list`: executam sobre o caminho resolvido e validado, respeitando as cotas de tamanho de arquivo (`limits.max_file_bytes`).
  - Para `filesystem.list`: aceita caminhos absolutos (ex: `path: "C:\\Users\\Nome\\Downloads"` ou `path: "D:\\Projetos"`), listando até `max_scan_entries`.

### 3.2 `apps/local-daemon`
- **Argumento `--full-computer`:**
  - Adicionado ao CLI do daemon (`Args`).
- **Validação de Diretório de Estado (`state_dir`):**
  - No modo `FullComputer`, o `state_dir` (normalmente em `LOCALAPPDATA`) está dentro do perfil do usuário (`root`).
  - O daemon aceita essa configuração porque `WorkspaceFiles` inclui explicitamente o `state_dir` no enclave estritamente protegido.

### 3.3 `apps/device-agent`
- **Perfil do Agente (`Profile` em `profile.rs`):**
  - Campo adicionado: `#[serde(default)] pub full_computer: bool`.
- **Validação de Perfil (`protect_profile`):**
  - Quando `full_computer: true`, não recusa se o perfil estiver dentro do workspace (pois o workspace é o perfil do usuário); em vez disso, assegura que o perfil está gravado com permissões privadas do Windows (ACL somente para o usuário).
- **Invocação do Daemon (`daemon::add_args`):**
  - Se `profile.full_computer`, passa a flag `--full-computer` ao daemon.

### 3.4 `tools/distribution/install.ps1` e `tools/setup-windows.ps1`
- **Fluxo Guiado de Instalação:**
  - Pergunta de consentimento explícito:
    - Opção 1: Computador Inteiro (Recomendado) -> `$Workspace = $env:USERPROFILE`, `$FullComputer = $true`.
    - Opção 2: Pasta Específica (Restrito) -> `$Workspace = Join-Path $env:USERPROFILE 'RemoteWorkspace'` ou seleção de pasta.
  - No pareamento: passa `--full-computer` para `device-agent pair`.
  - Exibe resumo claro da segurança ativada antes de conectar.

---

## 4. Plano de Testes e Validação

1. **Testes Unitários em Rust (`infrastructure` & `daemon`):**
   - Leitura de arquivo em caminho relativo ao perfil do usuário.
   - Leitura de arquivo com caminho absoluto (`C:\...`).
   - Tentativa de leitura de credenciais (`LOCALAPPDATA\RemoteDesktopController\agent\agent.json`) -> deve retornar `Policy / Denied`.
   - Tentativa de leitura de chave privada (`.ssh/id_rsa`) -> deve retornar `Policy / Denied`.
   - Tentativa de remoção de raiz do sistema ou do perfil -> deve retornar `Policy / Denied`.
   - Modo restrito continua rejeitando acessos fora da pasta do workspace.
2. **Testes de Integração com Agente e Pareamento:**
   - Pareamento com `--full-computer` inicializa o daemon corretamente.
   - Heartbeat do agente reporta root e escopo corretos.
3. **Teste do Instalador PowerShell:**
   - Instalação interativa e não-interativa no Windows validam as duas opções.

---

## 5. Compatibilidade e Migração

- Usuários com instalações existentes no modo restrito continuam funcionando sem interrupção.
- Para habilitar o modo Computador Inteiro em computadores já pareados: o instalador ou um comando simples de reconfiguração do agente atualiza a flag `full_computer` no perfil e reinicia o daemon.
