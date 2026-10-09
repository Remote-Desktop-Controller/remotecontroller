# Remote Controller Windows — Distribuição Oficial

Repositório público oficial de distribuição do **Remote Controller** para Windows x64.

Este repositório contém os instaladores, manifestos de integridade criptográfica SHA-256 e notas de versão estáveis. O código-fonte principal do projeto permanece privado.

---

## Como Instalar (Comando Único)

Abra o PowerShell no seu Windows e cole o comando oficial abaixo:

```powershell
irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 | iex
```

### O que o instalador faz automaticamente:
1. **Verificação SHA-256:** Baixa o pacote oficial do GitHub Releases e valida os hashes criptográficos de cada executável antes de qualquer inicialização.
2. **Instalação Local:** Instala em `%LOCALAPPDATA%\Programs\RemoteDesktopController` sem necessidade de permissões de administrador.
3. **Seleção de Pasta e Nome:** Permite escolher qual pasta de trabalho a IA poderá acessar e qual o nome deste computador no seu painel.
4. **Pareamento Guiado:** Inicia o pareamento com código de uso único e abre automaticamente seu navegador para confirmação no portal.
5. **Segundo Plano & Autostart:** Inicia o agente em segundo plano e oferece opção de inicialização automática ao ligar o computador.
6. **Segurança por Padrão:** Por padrão, o agente opera em **somente leitura**. Escrita, execução de processos e terminal permanecem desativados até que você os autorize explicitamente.

---

## Verificação Manual de Integridade (SHA-256)

Caso deseje verificar o hash manualmente antes de executar:

```powershell
# Verificar o hash do pacote zip baixado
(Get-FileHash remote-desktop-controller-windows-x64-*.zip -Algorithm SHA256).Hash
```

Compare a saída com o arquivo `.zip.sha256` e com o `manifest.json` publicado no release.

---

## Atualização e Desinstalação

### Atualizar para a versão estável mais recente
Basta executar novamente o instalador ou passar o parâmetro de atualização:
```powershell
irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 | iex
```
*(Suas credenciais e pareamento existentes serão preservados automaticamente)*

### Desinstalar
Para remover o Remote Controller do seu computador:
```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\RemoteDesktopController\uninstall.ps1"
```
O desinstalador encerra os processos em execução, remove a inicialização automática e limpa os arquivos do sistema.
