# Instalar o produto remoto

O pacote do produto contém `local-daemon`, `mcp-gateway`, `device-agent` e `remote-service`, manifesto SHA256, instalador Windows e licenças dos patches libSQL/portable-pty. O agente usa o daemon local para aplicar permissões. O serviço de contas/MCP é implantado separadamente no host HTTPS; incluí-lo no ZIP não abre portas no PC.

Não é necessário instalar Rust, Node, Python ou Docker no computador do usuário. O ZIP precisa corresponder ao sistema/arquitetura. Linux/macOS usam os mesmos binários sem `.exe`; o script Windows não instala nesses sistemas. Pacote preparado não comprova assinatura, publicação ou funcionamento no ChatGPT.

## Windows: conferir, instalar e parear

Compare o hash do ZIP com o checksum fornecido por uma origem confiável e extraia-o. O manifesto interno detecta corrupção; não autentica o editor. Use PowerShell 5.1 ou 7 para o instalador. `package-product.ps1`, usado pelo mantenedor, requer PowerShell 7.

```powershell
Get-FileHash -LiteralPath '.\remote-desktop-controller-windows-x64-0.3.0.zip' -Algorithm SHA256
Expand-Archive -LiteralPath '.\remote-desktop-controller-windows-x64-0.3.0.zip' -DestinationPath '.\rdc-package'
.\rdc-package\setup-windows.ps1 -PackageDirectory '.\rdc-package' -VerifyOnly
.\rdc-package\setup-windows.ps1 -PackageDirectory '.\rdc-package' -ServerUrl 'https://seu-dominio-real' -Workspace 'C:\Projetos\MeuProjeto' -Name 'Meu PC' -EnableAutostart -StartAgent
```

Substitua o hostname e a pasta pelos valores reais. Sem ServerUrl, o script somente instala. Sem AllowWrite, o pareamento cria um perfil de leitura. Para autorizar edição da pasta, acrescente `-AllowWrite`. Uma janela de seleção da pasta é opcional: use `powershell.exe -STA -File .\rdc-package\setup-windows.ps1 -ServerUrl https://seu-dominio-real -ChooseWorkspace`.

O agente imprime um endereço HTTPS e código de pareamento. Abra o endereço, entre/cadastre sua conta e aprove esse código. Credenciais do dispositivo ficam somente no perfil privado. Se a entrega única da credencial se perder antes de salvar, execute um novo pareamento. Não copie tokens para o chat.

Arquivos são copiados para `%LOCALAPPDATA%\Programs\RemoteDesktopController\<versão>`. Perfil e estado usam `%LOCALAPPDATA%\RemoteDesktopController\agent`; ambos ficam fora do workspace. InstallRoot pode ser outra subpasta de LOCALAPPDATA; ProfileDirectory permite outro perfil privado separado. Links/junctions no caminho são recusados. Perfil existente não é substituído: servidor, workspace e permissões anteriores são preservados mesmo que novos argumentos sejam fornecidos. Para outro computador/workspace, escolha outro ProfileDirectory e pareie separadamente.

`-VerifyOnly` confere manifesto/arquivos e termina sem copiar, abrir diálogo, parear, iniciar processos ou alterar registro/autostart. A instalação normal verifica checksums antes e depois da cópia. Recusa conteúdo diferente sob uma versão já instalada; use nova versão para atualizar. Pastas anteriores são conservadas. Nenhum binário é adicionado ao PATH e o serviço remoto não é iniciado.

## Iniciar, parar e desfazer autostart

```powershell
$agent = Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController\0.3.0\device-agent.exe'
$profile = Join-Path $env:LOCALAPPDATA 'RemoteDesktopController\agent'
& $agent --profile $profile run
& $agent --profile $profile status
& $agent --profile $profile stop
& $agent --profile $profile autostart enable
& $agent --profile $profile autostart disable
```

`run` em primeiro plano permite observar erros e encerrar com Ctrl+C. `-StartAgent` no instalador inicia oculto e informa o PID; confirme presença na conta e consulte status. Autostart Windows adiciona somente uma entrada do usuário atual e executa oculto no próximo login; `enable` não inicia imediatamente. `disable` remove essa entrada, mas não para um agente já ativo: use stop. O instalador oferece `-EnableAutostart`/`-DisableAutostart` para delegar essas mesmas operações. Não requer UAC.

Antes de instalar nova versão, pare o agente e seu daemon. Instale o novo pacote, confira status e reative autostart usando o novo executável; o registro conserva o perfil e passa a apontar para a nova versão. Não remova uma pasta de versão ainda usada no autostart. Para retirar do PC, desative autostart, execute stop e revogue o dispositivo na conta. Conserve o perfil até decidir conscientemente como arquivar seu histórico; o instalador não exclui estado.

## Processos e terminal: autorização local

Processos e shell não são liberados pelo pareamento padrão. Para perfis que precisam de processos, os switches explícitos são `-AllowProcess`, `-AllowProcessNetwork` e `-AllowProcessOutsideWorkspace`; programas/argumentos ainda precisam da aprovação local:

```powershell
& $agent --profile $profile approve --program 'C:\Tools\programa.exe' --trust-unconfined -- argumento1 argumento2
& $agent --profile $profile stop
& $agent --profile $profile terminal-approve --program 'C:\Windows\System32\cmd.exe' --trust-unconfined -- /Q
& $agent --profile $profile run
```

O terminal tem os privilégios, acesso a arquivos e rede do usuário do Windows. TerminalApprove fixa programa/argv/SHA256 e atualiza permissões via CLI, com agente parado. Não há aprovação genérica de shell pelo ChatGPT. A política e as regras de processos ficam fora do workspace e a mudança de executável exige nova aprovação.

## Linux/macOS

Extraia o pacote correspondente e mantenha-o em uma pasta permanente fora do workspace. O ZIP não preserva necessariamente permissões executáveis:

```sh
chmod +x local-daemon mcp-gateway device-agent remote-service
./device-agent pair --server https://seu-dominio-real --workspace /home/usuario/projeto --name 'Meu PC'
./device-agent run
./device-agent status
./device-agent stop
./device-agent autostart enable
./device-agent autostart disable
```

O perfil padrão fica em `$HOME/.local/share/remotecontroller/agent`; `--profile` escolhe outro diretório. Linux usa systemd do usuário; macOS usa LaunchAgent. A prova local Windows não comprova instalação/autostart nessas plataformas: consulte os gates de release correspondentes. As mutações Unix exigem escritores cooperativos durante operação/recovery; não oferecem rename condicionado ao inode contra troca concorrente externa.

## Preparar pacote como mantenedor

Primeiro execute os gates e compile os quatro binários no mesmo diretório, com o mesmo IPC. O script não compila nem dispara Cargo:

```powershell
.\tools\package-product.ps1 -BinaryDirectory 'C:\build\release' -OutputDirectory '.\dist' -Version 0.3.0 -Platform windows -Architecture x64 -VerifyOnly
.\tools\package-product.ps1 -BinaryDirectory 'C:\build\release' -OutputDirectory '.\dist' -Version 0.3.0 -Platform windows -Architecture x64
```

VerifyOnly é somente leitura também no empacotador. A versão aqui identifica a distribuição; o runtime local mantém sua versão CLI própria. `tools/package.ps1` e seu instalador do runtime core continuam independentes. Deploy, callback e publicação são descritos em [manual-owner.md](manual-owner.md).
