# Segurança, Fronteira de Confiança e Modelo de Ameaças — v0.5.0

O Remote Desktop Controller (RDC) adota o princípio de **mínimo privilégio estrito**, **consentimento explícito do operador**, e **isolamento físico de dados sensíveis fora do workspace gerenciado**.

---

## 1. Modos de Operação e Capabilities

Por padrão, ao iniciar o daemon sem parâmetros adicionais, o runtime concede exclusivamente:
- `Capability::ReadWorkspace`: Leitura de arquivos confinados ao workspace.
- `Capability::GitRead`: Inspeção de status, histórico e diffs do Git local.

### 1.1 Modo Confinado ao Workspace
Para habilitar modificações e processos restritos:
- `--allow-write`: Concede `Capability::WriteWorkspace`.
- `--allow-process`: Concede `Capability::SpawnProcess` e `Capability::KillProcess`. Exige regras estritas de executáveis e argumentos no arquivo de configuração JSON.
- `--allow-process-outside-workspace`: Autoriza o processo a operar fora da raiz do workspace.
- `--allow-process-network`: Permite que os processos filhos realizem chamadas de rede.

### 1.2 Modo Full Computer com Consentimento Explícito (`--full-computer`)
Introduzido no RFC-0003, o parâmetro `--full-computer` concede em uma única flag auditável:
`WriteWorkspace`, `SpawnProcess`, `KillProcess`, `OutsideWorkspaceAccess` e `NetworkAccess`.
- **Requisito Ético e de Consentimento:** O instalador interativo e as ferramentas CLI exigem confirmação explícita do operador humano para ativar essa modalidade.
- **Sem Shells Genéricos Cegos:** Mesmo em `--full-computer`, o runtime **rejeita strings de shell soltas** (`sh -c` ou `cmd.exe /c`). Cada execução é despachada com caminho de executável verificado, vetor estruturado de argumentos e variáveis de ambiente explícitas.

---

## 2. Enclave de Estado e Proteção Criptográfica DPAPI

### 2.1 Separação Física do Workspace
O diretório de dados persistentes do RDC **nunca reside dentro do diretório do projeto ou repositório trabalhado**:
- **Windows:** `%LOCALAPPDATA%\LocalRuntime\<workspace-hash>\`
- **Linux/macOS:** Diretórios privados `$XDG_DATA_HOME` / `~/Library/Application Support`.

Mesmo que um script ou comando execute um `git clean -fdx` ou remova recursivamente o conteúdo do workspace, o histórico transacional, chaves de autenticação e logs de auditoria permanecem intactos.

### 2.2 Cofre DPAPI Criptográfico no Windows (`storage::vault`)
Tokens de autenticação remota e segredos locais são cifrados no Windows com a API nativa DPAPI (`CryptProtectData` e `CryptUnprotectData` via `windows::Win32_Security_Cryptography`):
- Os segredos são protegidos com as credenciais criptográficas da conta de usuário logada no sistema operacional.
- Nenhum outro processo executando sob credenciais de outro usuário tem a capacidade de decifrar o arquivo de estado, mesmo tendo acesso físico aos blocos do disco.

---

## 3. Segurança IPC e Isolamento de Named Pipes

No Windows, todos os Named Pipes (`\\.\pipe\local-runtime-*`, `\\.\pipe\rdc-agent-maestro`, `\\.\pipe\rdc-telemetry`) são criados com descritores de segurança rígidos (DACL):
- Acesso restrito estritamente ao usuário proprietário e ao `NT AUTHORITY\SYSTEM`.
- A flag `first: true` na inicialização impede ataques de hijacking (onde um processo não confiável criaria o pipe antes do daemon legítimo).
- O pipe rejeita conexões de clientes remotos na rede local (`PIPE_REJECT_REMOTE_CLIENTS`).
- O handshake exige a troca de um segredo criptográfico efêmero de 32 bytes gerado com entropia de sistema.

---

## 4. Orquestrador RDC Maestro e Gate Humano (*Human-in-the-Loop*)

1. **Separação de Contexto de Navegação:** As janelas do WebView2 utilizam perfis locais isolados em `%LOCALAPPDATA%\rdc-maestro\EBWebView`, garantindo que scripts maliciosos de páginas da web não contaminem outras sessões.
2. **Preload Sanitizado:** O script `preload.js` não expõe ponteiros globais sensíveis na janela do navegador e valida estritamente a origem do documento (executando apenas no top-level frame).
3. **Canal de Ordens Aprovadas (`\\.\pipe\rdc-agent-maestro`):** Planos de automação sugeridos por LLMs (ChatGPT, Claude, Gemini, DeepSeek, Qwen) permanecem em rascunho visual no Blackboard. Nenhuma ação atinge o kernel do computador sem o clique de aprovação explícito do operador humano.

---

## 5. Desinstalador Seguro (*Scoped Uninstaller*)

Tanto o instalador quanto o desinstalador (`tools/distribution/uninstall.ps1` e `uninstall.ps1`) implementam verificações de segurança que blindam o sistema contra deleções acidentais:
- Rejeição absoluta caso o caminho informado seja a raiz de volume (`C:\`), pasta de sistema (`System32`, `Windows`, `Program Files`) ou a raiz da pasta do usuário (`C:\Users\<user>`).
- Encerramento cirúrgico por PID apenas dos processos do produto iniciados a partir do diretório de instalação, evitando terminação de processos de terceiros ou da IDE.
