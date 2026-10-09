# Evidência de Validação: Distribuição Pública e Instalador Windows

Data: 2026-10-09  
Ambiente: Windows 11 x64  
Branch: `feat/commercial-portal`  
Repositório Público Alvo: `Remote-Desktop-Controller/remotecontroller-windows`  
Servidor Oficial: `https://andredozero.webajato.com.br`  

---

## 1. Resumo da Implementação

1. **Pipeline de Release Windows (`.github/workflows/release-windows.yml`):**
   - Disparo por tag (`v*`) ou manual (`workflow_dispatch`).
   - Compilação `--release` em `windows-latest` para `device-agent` e `local-daemon`.
   - Empacotamento com manifesto `manifest.json` contendo hashes SHA-256 por arquivo e `remote-desktop-controller-windows-x64-*.zip.sha256`.
   - Suporte preparado para assinatura digital Authenticode via secrets (`CODE_SIGNING_CERT_BASE64` e `CODE_SIGNING_PASSWORD`).
   - Publicação exclusiva no repositório público de distribuição sem expor o código-fonte privado.

2. **Instalador Oficial (`tools/distribution/install.ps1`):**
   - Comando de execução única:
     ```powershell
     irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 | iex
     ```
   - Compatível com Windows PowerShell 5.1 e PowerShell 7.
   - Validação criptográfica obrigatória do hash SHA-256 do arquivo ZIP antes da extração.
   - Validação do SHA-256 individual de cada executável contra o manifesto oficial.
   - Instalação no diretório de usuário `%LOCALAPPDATA%\Programs\RemoteDesktopController\<version>` (sem privilégios de administrador/UAC).
   - Seletor de pasta de trabalho (`Workspace`) e prompt de nome do computador (padrão `$env:COMPUTERNAME`).
   - Política de segurança restritiva por padrão: **somente leitura** (sem permissão de escrita, processos arbitrários ou terminal).
   - Pareamento automático com abertura imediata do navegador na URL de confirmação do portal.
   - Prompt para consentimento de inicialização automática ao ligar o computador (registro `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).
   - Execução do agente em segundo plano sem necessidade de manter o PowerShell aberto.

3. **Desinstalador Limpo (`tools/distribution/uninstall.ps1`):**
   - Encerra processos ativos (`device-agent` e `local-daemon`).
   - Remove a chave de inicialização automática do registro do usuário.
   - Permite escolher entre manter ou remover credenciais de pareamento locais (`agent.json`).
   - Remove todos os binários da pasta de instalação.

4. **Dashboard e Portal Comercial (`apps/portal`):**
   - Componente `WindowsInstallerCard` integrado ao Dashboard e à página `/installation`.
   - Botão para copiar o comando oficial com feedback visual imediato ("Copiado!").
   - Passo a passo visual de 3 etapas para usuários leigos.
   - Tutorial de Conexão MCP ao ChatGPT com a URL oficial `https://andredozero.webajato.com.br/mcp`, instruções de autenticação OAuth e aviso de segurança sobre modo somente leitura.

---

## 2. Evidências dos Testes Automatizados

### Teste do Instalador, Integridade SHA-256 e Desinstalação (`tools/test-distribution-installer.ps1`)
Comando: `powershell -ExecutionPolicy Bypass -File tools/test-distribution-installer.ps1`
Resultado:
- Teste: Estrutura do pacote e manifesto SHA-256 válidos ... **PASSOU**
- Teste: Detecção e rejeição imediata de pacote zip adulterado (Checksum Divergente) ... **PASSOU**
- Teste: Rejeição de binário interno adulterado pós-extração ... **PASSOU**
- Teste: Instalação bem-sucedida de executáveis e criação do desinstalador ... **PASSOU**
- Teste: Execução do desinstalador remove diretórios de instalação ... **PASSOU**
**Total: 5 Passaram | 0 Falharam**

### Testes da Interface e Integração do Portal (`apps/portal`)
1. **Vitest (`npm test`):**
   - 18 arquivos de teste executados.
   - **161 testes aprovados, 2 skipped, 0 falhas**.
   - Inclui `tests/installer-onboarding.test.tsx` (validação da renderização do comando PowerShell, avisos SHA-256 e tutorial MCP ChatGPT).
2. **Typecheck (`npm run typecheck`):**
   - `tsc --noEmit` concluído com código de saída 0 (zero erros de tipo).
3. **Lint (`npm run lint`):**
   - `eslint .` concluído com código de saída 0 (zero erros).
4. **Build (`npm run build`):**
   - Next.js webpack production build compilado em 61 segundos.
   - 28 rotas estáticas e dinâmicas geradas com sucesso.

### Testes dos Binários Nativos Rust (`cargo test`)
1. `cargo test -p device-agent`:
   - `budget::budget_tests` ... **ok**
   - `daemon::tests` ... **ok**
   - `remote_flow` (fluxo de pareamento real, escrita, leitura, renovação, reinício) ... **ok**
   - `runner_regressions` (7 cenários) ... **ok**
   - `security` (7 cenários de segurança) ... **ok**
2. `cargo test -p local-daemon`:
   - `host`, `install` (4 cenários) ... **ok**
   - `config_security` (3 cenários) ... **ok**
   - `fullstack` (3 cenários de integração MCP/daemon com reinício abrupto e recuperação) ... **ok**

---

## 3. Estado Atual e Segurança Operacional

- **Repositório público de distribuição:** O repositório `Remote-Desktop-Controller/remotecontroller-windows` foi estruturado e documentado em `tools/distribution/`. Nenhuma publicação remota foi executada, conforme exigido.
- **Ambiente de produção:** Nenhuma migration foi executada em produção, a VPS não foi reiniciada e o plano Free não foi ativado sem autorização prévia.
- **Licenciamento e Asaas:** Regras de licenciamento, webhook e chaves financeiras foram estritamente preservadas.
