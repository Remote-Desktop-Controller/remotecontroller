#requires -Version 5.1
<#
.SYNOPSIS
    Instalador oficial do Remote Controller Windows.
    Baixa releases verificadas, valida integridade criptográfica SHA-256 e guia o pareamento.
#>
[CmdletBinding()]
param(
    [string]$Version = 'latest',
    [string]$Repo = 'Remote-Desktop-Controller/remotecontroller-windows',
    [uri]$ServerUrl = [uri]'https://andredozero.webajato.com.br',
    [string]$Workspace,
    [string]$Name,
    [switch]$FullComputer,
    [switch]$AllowWrite,
    [switch]$AllowProcess,
    [switch]$AllowTerminal,
    [switch]$EnableAutostart,
    [switch]$DisableAutostart,
    [switch]$Update,
    [string]$LocalPackagePath,
    [string]$InstallDir,
    [string]$ProfileDir,
    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-Step([string]$Message) {
    Write-Host "`n==> $Message" -ForegroundColor Cyan
}

function Write-Success([string]$Message) {
    Write-Host " [OK] $Message" -ForegroundColor Green
}

function Write-WarnMessage([string]$Message) {
    Write-Host " [!] $Message" -ForegroundColor Yellow
}

function Assert-NoReparsePath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    while ($fullPath) {
        if (Test-Path -LiteralPath $fullPath) {
            $item = Get-Item -LiteralPath $fullPath -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Links ou junctions não são permitidos por segurança: $fullPath"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($fullPath)
        if ($parent -eq $fullPath) { break }
        $fullPath = $parent
    }
}

function Test-Within([string]$Path, [string]$Parent) {
    $fullPath = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $fullParent = [IO.Path]::GetFullPath($Parent).TrimEnd('\', '/')
    if ($fullPath.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { $fullPath = '\\' + $fullPath.Substring(8) }
    elseif ($fullPath.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { $fullPath = $fullPath.Substring(4) }
    if ($fullParent.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { $fullParent = '\\' + $fullParent.Substring(8) }
    elseif ($fullParent.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { $fullParent = $fullParent.Substring(4) }
    return $fullPath.Equals($fullParent, [StringComparison]::OrdinalIgnoreCase) -or $fullPath.StartsWith($fullParent + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

function Get-MachineArchitecture {
    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    if ($arch -eq 'AMD64') { return 'x64' }
    if ($arch -eq 'ARM64') { return 'arm64' }
    throw "Arquitetura Windows não suportada: $arch. Necessário x64 ou arm64."
}

# 1. Validação de Plataforma
Write-Host "==========================================================" -ForegroundColor Magenta
Write-Host "        Remote Controller - Instalador Oficial Windows     " -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Magenta

if ($env:OS -ne 'Windows_NT' -or -not $env:LOCALAPPDATA) {
    throw 'A instalação exige sistema operacional Windows e acesso a LOCALAPPDATA.'
}
$architecture = Get-MachineArchitecture
if ($architecture -ne 'x64') {
    throw "No momento esta versão oferece binários para Windows x64. Arquitetura detectada: $architecture."
}

# 2. Definição de Diretórios Seguros (Sem necessidade de Admin)
$installBase = if ($InstallDir) { $InstallDir } else { Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController' }

$userHome = if ($HOME -and $HOME.Trim()) {
    $HOME
} elseif ($env:USERPROFILE -and $env:USERPROFILE.Trim()) {
    $env:USERPROFILE
} else {
    [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::UserProfile)
}
$canonicalProfile = if ($ProfileDir) { $ProfileDir } else { Join-Path $userHome '.remotecontroller\agent' }
$legacyProfile = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'RemoteDesktopController\agent' } else { $null }

# Harmonização de diretório de perfil com compatibilidade retroativa
if (-not $ProfileDir -and $legacyProfile -and (Test-Path -LiteralPath (Join-Path $legacyProfile 'agent.json')) -and -not (Test-Path -LiteralPath (Join-Path $canonicalProfile 'agent.json'))) {
    $profileBase = $legacyProfile
    Write-WarnMessage "Perfil legado com credenciais detectado em '$legacyProfile'. Mantendo compatibilidade retroativa."
} else {
    $profileBase = $canonicalProfile
}

Assert-NoReparsePath $installBase
Assert-NoReparsePath $profileBase

New-Item -ItemType Directory -Path $installBase -Force | Out-Null
New-Item -ItemType Directory -Path $profileBase -Force | Out-Null

$tempStaging = Join-Path $env:TEMP ("rdc-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempStaging -Force | Out-Null

try {
    # 3. Obtenção do Pacote e Verificação Criptográfica
    Write-Step "Baixando pacote verificado e manifesto oficial..."
    $manifest = $null
    $packageDir = $null

    if ($LocalPackagePath) {
        # Modo de teste local / airgap
        $resolvedLocal = (Resolve-Path -LiteralPath $LocalPackagePath).Path
        if (Test-Path -LiteralPath (Join-Path $resolvedLocal 'manifest.json')) {
            $packageDir = $resolvedLocal
        } elseif ($resolvedLocal.EndsWith('.zip', [StringComparison]::OrdinalIgnoreCase)) {
            Expand-Archive -LiteralPath $resolvedLocal -DestinationPath $tempStaging -Force
            $packageDir = $tempStaging
        } else {
            throw "LocalPackagePath deve apontar para uma pasta com manifest.json ou arquivo .zip: $LocalPackagePath"
        }
    } else {
        # Download HTTPS oficial do GitHub Releases
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.SecurityProtocolType]::Tls13
        
        $resolvedTag = $Version
        if ($resolvedTag -eq 'latest') {
            try {
                $releaseApiUrl = "https://api.github.com/repos/$Repo/releases/latest"
                $headers = @{ 'User-Agent' = 'RemoteController-Installer/1.0' }
                $releaseInfo = Invoke-RestMethod -Uri $releaseApiUrl -Headers $headers -TimeoutSec 15
                $resolvedTag = $releaseInfo.tag_name
            } catch {
                Write-WarnMessage "Consulta à API do GitHub indisponível; usando versão padrão estável v0.5.0"
                $resolvedTag = "v0.5.0"
            }
        }
        
        $cleanVer = $resolvedTag.TrimStart('v')
        $zipName = "remote-desktop-controller-windows-x64-$cleanVer.zip"
        $downloadBase = "https://github.com/$Repo/releases/download/$resolvedTag"
        
        $zipUrl = "$downloadBase/$zipName"
        $shaUrl = "$zipUrl.sha256"
        $manifestUrl = "$downloadBase/manifest.json"
        
        $localZip = Join-Path $tempStaging $zipName
        $localSha = Join-Path $tempStaging "$zipName.sha256"
        $localManifest = Join-Path $tempStaging 'manifest.json'
        
        Write-Host "Baixando $zipName de $downloadBase..." -ForegroundColor Gray
        Invoke-WebRequest -Uri $zipUrl -OutFile $localZip -UseBasicParsing -TimeoutSec 60
        Invoke-WebRequest -Uri $shaUrl -OutFile $localSha -UseBasicParsing -TimeoutSec 15
        
        # Validação do hash SHA-256 do arquivo ZIP antes da extração
        $expectedZipHash = (Get-Content -LiteralPath $localSha -Raw).Trim().Split(' ')[0].ToLowerInvariant()
        $actualZipHash = (Get-FileHash -LiteralPath $localZip -Algorithm SHA256).Hash.ToLowerInvariant()
        
        if ($actualZipHash -ne $expectedZipHash) {
            throw "FALHA DE SEGURANÇA: Checksum SHA-256 do arquivo baixado ($actualZipHash) não corresponde ao oficial ($expectedZipHash)!"
        }
        Write-Success "Integridade SHA-256 do pacote validada com sucesso: $actualZipHash"
        
        Expand-Archive -LiteralPath $localZip -DestinationPath $tempStaging -Force
        $packageDir = $tempStaging
    }

    # Validação rigorosa do manifest.json e arquivos individuais
    $manifestPath = Join-Path $packageDir 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        throw "manifest.json ausente no pacote de instalação."
    }
    
    $manifestContent = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifestContent.product -ne 'remote-desktop-controller' -or $manifestContent.platform -ne 'windows') {
        throw "Manifesto inválido ou não pertencente ao Remote Controller Windows."
    }

    Write-Step "Validando checksums individuais dos executáveis..."
    foreach ($fileProp in $manifestContent.files.PSObject.Properties) {
        $fileName = $fileProp.Name
        $expectedHash = $fileProp.Value.ToLowerInvariant()
        $filePath = Join-Path $packageDir $fileName
        
        if (-not (Test-Path -LiteralPath $filePath)) {
            throw "Arquivo obrigatório ausente no pacote: $fileName"
        }
        $actualHash = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne $expectedHash) {
            throw "FALHA DE INTEGRIDADE: O arquivo $fileName foi adulterado ou corrompido! (Esperado: $expectedHash, Obtido: $actualHash)"
        }
    }
    Write-Success "Todos os executáveis e bibliotecas foram verificados criptograficamente."

    # 4. Instalação / Atualização em Diretório de Versão
    $versionRoot = Join-Path $installBase $manifestContent.version
    Assert-NoReparsePath $versionRoot
    
    Write-Step "Instalando binários em $versionRoot..."
    if (Test-Path -LiteralPath $versionRoot) {
        Write-WarnMessage "A versão $($manifestContent.version) já está instalada."
    } else {
        New-Item -ItemType Directory -Path $versionRoot -Force | Out-Null
        foreach ($fileProp in $manifestContent.files.PSObject.Properties) {
            $destFile = Join-Path $versionRoot $fileProp.Name
            $destDir = [IO.Path]::GetDirectoryName($destFile)
            if (-not (Test-Path -LiteralPath $destDir)) {
                New-Item -ItemType Directory -Path $destDir -Force | Out-Null
            }
            Copy-Item -LiteralPath (Join-Path $packageDir $fileProp.Name) -Destination $destFile -Force
        }
        Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $versionRoot 'manifest.json') -Force
    }
    
    $taskAgent = Join-Path $versionRoot 'device-agent.exe'
    $taskDaemon = Join-Path $versionRoot 'local-daemon.exe'
    $taskMonitor = Join-Path $versionRoot 'rdc-monitor.exe'
    
    if (-not (Test-Path -LiteralPath $taskAgent) -or -not (Test-Path -LiteralPath $taskDaemon)) {
        throw "Executáveis essenciais não encontrados após a instalação."
    }

    # Criar / atualizar script de desinstalação na raiz do programa
    $uninstallScriptPath = Join-Path $installBase 'uninstall.ps1'
    $uninstallContent = @'
#requires -Version 5.1
[CmdletBinding()]
param([string]$InstallPath, [string]$ProfilePath, [switch]$RemoveData, [switch]$NonInteractive)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "        Remote Controller - Desinstalador Windows         " -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Cyan

$installRoot = if ($InstallPath) {
    $InstallPath
} elseif (-not [string]::IsNullOrWhiteSpace($PSScriptRoot) -and (Split-Path -Path $PSScriptRoot -Leaf) -eq 'RemoteDesktopController') {
    $PSScriptRoot
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController'
}

$installRoot = [IO.Path]::GetFullPath($installRoot).TrimEnd('\', '/')
$installLeaf = Split-Path -Path $installRoot -Leaf
if ($installLeaf -ne 'RemoteDesktopController') {
    throw "ERRO CRÍTICO DE SEGURANÇA: O diretório alvo de desinstalação deve se chamar 'RemoteDesktopController'. Alvo recebido: '$installRoot'."
}

$forbiddenCriticalPaths = @(
    $env:WINDIR,
    $env:SystemRoot,
    $env:ProgramFiles,
    ${env:ProgramFiles(x86)},
    $env:USERPROFILE,
    $env:LOCALAPPDATA,
    (Join-Path $env:LOCALAPPDATA 'Programs')
) | Where-Object { $_ } | ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\', '/') }

if ($forbiddenCriticalPaths -contains $installRoot) {
    throw "ERRO CRÍTICO DE SEGURANÇA: O diretório de instalação coincide com um caminho de sistema ou raiz compartilhada ('$installRoot')."
}

$userHome = if ($HOME -and $HOME.Trim()) {
    $HOME
} elseif ($env:USERPROFILE -and $env:USERPROFILE.Trim()) {
    $env:USERPROFILE
} else {
    [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::UserProfile)
}
$canonicalProfile = Join-Path $userHome '.remotecontroller\agent'
$legacyProfile = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'RemoteDesktopController\agent' } else { $null }

$profileRoot = if ($ProfilePath) {
    $ProfilePath
} elseif ($legacyProfile -and (Test-Path -LiteralPath (Join-Path $legacyProfile 'agent.json')) -and -not (Test-Path -LiteralPath (Join-Path $canonicalProfile 'agent.json'))) {
    $legacyProfile
} else {
    $canonicalProfile
}

# 1. Parar agentes ativos
Get-Process -Name 'device-agent', 'local-daemon', 'rdc', 'mcp-gateway', 'rdc-monitor' -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "Encerrando processo $($_.Name) (PID $($_.Id))..." -ForegroundColor Yellow
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}

# 2. Remover chave de autostart
try {
    $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
    Get-ItemProperty -Path $runKey -ErrorAction SilentlyContinue | Get-Member -MemberType NoteProperty | Where-Object { $_.Name -like 'remotecontroller-*' } | ForEach-Object {
        Remove-ItemProperty -Path $runKey -Name $_.Name -Force -ErrorAction SilentlyContinue
        Write-Host "Inicialização automática removida do registro." -ForegroundColor Green
    }
} catch {}

# 3. Dados do usuário
if (-not $RemoveData -and -not $NonInteractive) {
    $resp = Read-Host "Deseja remover as credenciais e configurações de pareamento locais? (S/N) [Padrão: N]"
    if ($resp -match '^(s|sim|y|yes)$') { $RemoveData = $true }
}
if ($RemoveData) {
    $profilesToPurge = @()
    if ($ProfilePath) {
        $profilesToPurge += $ProfilePath
    } else {
        $profilesToPurge += $canonicalProfile
        if ($legacyProfile) { $profilesToPurge += $legacyProfile }
    }
    foreach ($p in $profilesToPurge) {
        if (Test-Path -LiteralPath $p) {
            Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction SilentlyContinue
            Write-Host "Perfil local removido com sucesso: $p" -ForegroundColor Green
        }
    }
} else {
    Write-Host "Perfil local preservado em $profileRoot." -ForegroundColor Gray
}

# 4. Remover executáveis
Start-Sleep -Seconds 1
if (Test-Path -LiteralPath $installRoot) {
    Get-ChildItem -Path $installRoot -Exclude 'uninstall.ps1' -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    try {
        Remove-Item -LiteralPath (Join-Path $installRoot 'uninstall.ps1') -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $installRoot -Force -Recurse -ErrorAction SilentlyContinue
    } catch {}
}
Write-Host ""
Write-Host "[OK] Remote Controller desinstalado com sucesso." -ForegroundColor Green
Write-Host ""
'@
    [IO.File]::WriteAllText($uninstallScriptPath, $uninstallContent, [Text.UTF8Encoding]::new($true))
    Write-Success "Script de desinstalação configurado em: $uninstallScriptPath"

    # 5. Configuração e Pareamento
    $credentialsPath = Join-Path $profileBase 'agent.json'
    
    if (Test-Path -LiteralPath $credentialsPath) {
        Write-Step "Perfil de computador pareado já existente detectado."
        Write-Host "Credenciais de pareamento preservadas com sucesso." -ForegroundColor Green
        
        # Na atualização, assegura que Controle Completo esteja ativo se solicitado ou por padrão
        $shouldEnableFull = $FullComputer
        if (-not $shouldEnableFull -and -not $NonInteractive) {
            $respUpgrade = Read-Host "Deseja assegurar Controle Completo (Escrita, Processos e Terminal) nesta instalação? (S/N) [Padrão: S]"
            if (-not $respUpgrade -or $respUpgrade -match '^(s|sim|y|yes)$') {
                $shouldEnableFull = $true
            }
        } elseif (-not $shouldEnableFull -and $NonInteractive) {
            $shouldEnableFull = $true
        }
        
        if ($shouldEnableFull) {
            try {
                $agentJson = Get-Content -LiteralPath $credentialsPath -Raw | ConvertFrom-Json
                $agentJson.full_computer = $true
                $agentJson.allow_write = $true
                $agentJson.allow_process = $true
                $agentJson.allow_terminal = $true
                $agentJson.allow_process_outside_workspace = $true
                $agentJson.allow_process_network = $true
                [IO.File]::WriteAllText($credentialsPath, ($agentJson | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
                Write-Success "Permissões de Controle Completo ativadas no perfil existente."
                
                $powerShellPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
                if (Test-Path -LiteralPath $powerShellPath) {
                    & "$taskAgent" --profile "$profileBase" --daemon "$taskDaemon" terminal-approve --program "$powerShellPath" --trust-unconfined
                    Write-Success "Terminal PowerShell aprovado para uso imediato pelo ChatGPT."
                }
            } catch {
                Write-WarnMessage "Aviso ao atualizar permissões do perfil: $_"
            }
        }
    } else {
        Write-Step "Configuração de Autorização e Pareamento"
        
        # Nome do computador
        if (-not $Name) {
            if ($NonInteractive) {
                $Name = if ($env:COMPUTERNAME) { $env:COMPUTERNAME } else { 'Meu Computador' }
            } else {
                $defaultName = if ($env:COMPUTERNAME) { $env:COMPUTERNAME } else { 'Meu Computador' }
                $inputName = Read-Host "Nome deste computador no portal [Padrão: $defaultName]"
                $Name = if ($inputName.Trim()) { $inputName.Trim() } else { $defaultName }
            }
        }
        Write-Host "Nome definido: $Name" -ForegroundColor Gray

        # Autorização do Modo de Controle
        if (-not $Workspace -and -not $FullComputer) {
            if ($NonInteractive) {
                $Workspace = $env:USERPROFILE
                $FullComputer = $true
                $AllowWrite = $true
                $AllowProcess = $true
                $AllowTerminal = $true
            } else {
                Write-Host "`n-- Autorização de Controle do Computador --" -ForegroundColor Cyan
                Write-Host "[1] Controle Completo do Computador (Recomendado):" -ForegroundColor Green
                Write-Host "    * Navegar e ler pastas e arquivos em todas as unidades (Downloads, Documentos, Projetos, etc.)" -ForegroundColor White
                Write-Host "    * Criar, editar, mover e gerenciar arquivos e projetos completos" -ForegroundColor White
                Write-Host "    * Executar ferramentas e programas locais autorizados" -ForegroundColor White
                Write-Host "    * Utilizar terminal remoto interativo (PowerShell)" -ForegroundColor White
                Write-Host "    * Segurança ativa: credenciais (.ssh, .aws) e arquivos de sistema permanecem protegidos." -ForegroundColor Gray
                Write-Host "[2] Modo Restrito (Apenas Leitura em Pasta Isolada):" -ForegroundColor Yellow
                Write-Host "    * Restringe o acesso exclusivamente a uma pasta de sua escolha." -ForegroundColor White
                Write-Host "    * Apenas leitura; escrita, processos e terminal desativados." -ForegroundColor Gray
                
                $modeChoice = Read-Host "`nEscolha o modo de controle [1 ou 2, Padrão: 1]"
                if ($modeChoice.Trim() -eq '2') {
                    $FullComputer = $false
                    $defaultWs = Join-Path $env:USERPROFILE 'RemoteWorkspace'
                    Write-Host "`nEscolha a pasta restrita autorizada." -ForegroundColor Yellow
                    $chosenWs = $null
                    if ([Threading.Thread]::CurrentThread.ApartmentState -eq 'STA') {
                        try {
                            Add-Type -AssemblyName System.Windows.Forms
                            $dialog = New-Object Windows.Forms.FolderBrowserDialog
                            $dialog.Description = 'Selecione a pasta de trabalho autorizada para o Remote Controller'
                            $dialog.ShowNewFolderButton = $true
                            if ($dialog.ShowDialog() -eq [Windows.Forms.DialogResult]::OK) {
                                $chosenWs = $dialog.SelectedPath
                            }
                            $dialog.Dispose()
                        } catch {}
                    }
                    if (-not $chosenWs) {
                        $inputWs = Read-Host "Digite o caminho completo da pasta [Padrão: $defaultWs]"
                        $chosenWs = if ($inputWs.Trim()) { $inputWs.Trim() } else { $defaultWs }
                    }
                    $Workspace = $chosenWs
                } else {
                    $FullComputer = $true
                    $AllowWrite = $true
                    $AllowProcess = $true
                    $AllowTerminal = $true
                    $Workspace = $env:USERPROFILE
                }
            }
        } elseif ($FullComputer) {
            $AllowWrite = $true
            $AllowProcess = $true
            $AllowTerminal = $true
            if (-not $Workspace) {
                $Workspace = $env:USERPROFILE
            }
        }
        
        $Workspace = [IO.Path]::GetFullPath($Workspace)
        if (-not (Test-Path -LiteralPath $Workspace)) {
            New-Item -ItemType Directory -Path $Workspace -Force | Out-Null
        }
        Assert-NoReparsePath $Workspace
        
        # Apresentação da Política Operacional Ativa
        Write-Host "`n-- Política Operacional Ativa --" -ForegroundColor Cyan
        if ($FullComputer) {
            Write-Host "Controle Completo autorizado: Leitura, Escrita, Processos e Terminal HABILITADOS." -ForegroundColor Green
            Write-Host "Workspace base: $Workspace" -ForegroundColor Gray
            Write-Host "Credenciais sensíveis (.ssh, .aws) continuam estritamente protegidas por enclave." -ForegroundColor Gray
        } else {
            Write-Host "Modo restrito: Apenas leitura restrita dentro de $Workspace." -ForegroundColor Yellow
        }

        # Iniciar pareamento guiado
        Write-Step "Iniciando pareamento com o servidor $ServerUrl..."
        
        $pairArgs = @(
            '--profile', $profileBase,
            '--daemon', $taskDaemon,
            'pair',
            '--server', $ServerUrl.AbsoluteUri.TrimEnd('/'),
            '--workspace', $Workspace,
            '--name', $Name
        )
        if ($FullComputer) {
            $pairArgs += @(
                '--full-computer',
                '--allow-write',
                '--allow-process',
                '--allow-terminal',
                '--allow-process-outside-workspace',
                '--allow-process-network'
            )
        } else {
            if ($AllowWrite) { $pairArgs += '--allow-write' }
            if ($AllowProcess) { $pairArgs += '--allow-process' }
            if ($AllowTerminal) { $pairArgs += '--allow-terminal' }
        }
        
        # Construção segura de argumentos com escape para linha de comando
        $escapedPairArgs = foreach ($arg in $pairArgs) {
            if ($arg -match '[\s"]') {
                '"' + ($arg -replace '"', '\"') + '"'
            } else {
                $arg
            }
        }
        $serverEndpoint = $ServerUrl.AbsoluteUri.TrimEnd('/')
        $manualPairParts = @(
            "& `"$taskAgent`"",
            "--profile `"$profileBase`""
        )
        if ($taskDaemon -and (Test-Path -LiteralPath $taskDaemon)) {
            $manualPairParts += "--daemon `"$taskDaemon`""
        }
        $manualPairParts += @(
            "pair",
            "--server `"$serverEndpoint`"",
            "--workspace `"$Workspace`"",
            "--name `"$Name`""
        )
        if ($FullComputer) {
            $manualPairParts += @(
                "--full-computer",
                "--allow-write",
                "--allow-process",
                "--allow-terminal",
                "--allow-process-outside-workspace",
                "--allow-process-network"
            )
        } else {
            if ($AllowWrite) { $manualPairParts += "--allow-write" }
            if ($AllowProcess) { $manualPairParts += "--allow-process" }
            if ($AllowTerminal) { $manualPairParts += "--allow-terminal" }
        }
        $manualPairCommand = ($manualPairParts -join ' ')

        Write-Host "Executando pareamento inicial..." -ForegroundColor Gray
        
        # Execução com captura de saída para exibir o código e abrir o navegador automaticamente
        $pairProcInfo = New-Object System.Diagnostics.ProcessStartInfo
        $pairProcInfo.FileName = $taskAgent
        $pairProcInfo.Arguments = ($escapedPairArgs -join ' ')
        $pairProcInfo.RedirectStandardOutput = $true
        $pairProcInfo.RedirectStandardError = $true
        $pairProcInfo.UseShellExecute = $false
        $pairProcInfo.CreateNoWindow = $true

        $pairProc = [System.Diagnostics.Process]::Start($pairProcInfo)
        $browserOpened = $false

        # Inicia leitura assíncrona de stderr para evitar deadlock em pipes de saída
        $stdErrTask = $pairProc.StandardError.ReadToEndAsync()

        while (-not $pairProc.StandardOutput.EndOfStream) {
            $line = $pairProc.StandardOutput.ReadLine()
            if ($line) {
                Write-Host $line -ForegroundColor White
                # Detecta URL de pareamento para abrir automaticamente o navegador
                if ($line -match 'Abra\s+(https?://[^\s]+)\s+e aprove o código\s+([A-Za-z0-9_-]+)') {
                    $verifyUrl = $matches[1]
                    $pairCode = $matches[2]
                    
                    Write-Host "`n========================================================" -ForegroundColor Green
                    Write-Host "  CODIGO DE PAREAMENTO: $pairCode" -ForegroundColor Yellow
                    Write-Host "  ABRA O PORTAL E CONFIRME: $verifyUrl" -ForegroundColor Cyan
                    Write-Host "========================================================`n" -ForegroundColor Green
                    
                    if (-not $browserOpened) {
                        try {
                            Start-Process $verifyUrl
                            $browserOpened = $true
                            Write-Host "Navegador aberto automaticamente na página de confirmação." -ForegroundColor Gray
                        } catch {}
                    }
                }
            }
        }
        $pairProc.WaitForExit()
        
        $errText = $stdErrTask.Result
        if ($pairProc.ExitCode -ne 0 -and $errText) {
            Write-WarnMessage "Processo de pareamento finalizou com código $($pairProc.ExitCode): $errText"
        }

        # Validação explícita se o arquivo agent.json foi de fato criado no diretório de perfil
        if (Test-Path -LiteralPath $credentialsPath) {
            Write-Success "Pareamento concluído com sucesso!"

            # Aprovação automática do terminal PowerShell para uso imediato pelo ChatGPT
            if ($FullComputer -or $AllowTerminal) {
                $powerShellPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
                if (Test-Path -LiteralPath $powerShellPath) {
                    Write-Host "Configurando aprovação local do PowerShell para o terminal..." -ForegroundColor Gray
                    & "$taskAgent" --profile "$profileBase" --daemon "$taskDaemon" terminal-approve --program "$powerShellPath" --trust-unconfined
                    Write-Success "Terminal PowerShell aprovado para uso imediato pelo ChatGPT."
                }
            }
        } else {
            Write-Host ""
            Write-Host "==========================================================" -ForegroundColor Red
            Write-Host "   ALERTA: PAREAMENTO NÃO CONCLUÍDO NO NAVEGADOR!         " -ForegroundColor Yellow
            Write-Host "==========================================================" -ForegroundColor Red
            Write-Host "O arquivo de credenciais '$credentialsPath' não foi criado." -ForegroundColor Yellow
            Write-Host "Possíveis causas: a janela do navegador foi fechada, o token expirou ou houve erro na confirmação." -ForegroundColor Yellow
            Write-Host "O agente em segundo plano NÃO será iniciado até que o pareamento seja concluído." -ForegroundColor Yellow
            Write-Host "`nPara concluir o pareamento quando estiver pronto, execute no PowerShell:" -ForegroundColor White
            Write-Host "  $manualPairCommand`n" -ForegroundColor Cyan
        }
    }

    # 6. Consentimento de Autostart
    Write-Step "Inicialização Automática com o Windows"
    $isPaired = Test-Path -LiteralPath $credentialsPath
    if ($isPaired) {
        $shouldAutostart = $false
        if ($EnableAutostart) {
            $shouldAutostart = $true
        } elseif ($DisableAutostart) {
            $shouldAutostart = $false
        } elseif ($NonInteractive) {
            $shouldAutostart = $false
        } else {
            $respAuto = Read-Host "Deseja que o Remote Controller inicie automaticamente ao ligar o computador? (S/N) [Padrão: S]"
            if (-not $respAuto -or $respAuto -match '^(s|sim|y|yes)$') {
                $shouldAutostart = $true
            }
        }

        if ($shouldAutostart) {
            & "$taskAgent" --profile "$profileBase" autostart enable
            Write-Success "Inicialização automática ativada no registro do usuário (sem privilégios administrativos)."
        } else {
            Write-Host "Inicialização automática não ativada. Você pode ativá-la quando quiser com: & '$taskAgent' autostart enable" -ForegroundColor Gray
        }
    } else {
        Write-Host "Inicialização automática postergada: requer pareamento prévio concluído." -ForegroundColor Gray
    }

    # 7. Iniciar o Agente em Segundo Plano
    if ($isPaired) {
        Write-Step "Iniciando o agente em segundo plano..."
        # Mata qualquer instância antiga presa antes de subir
        Get-Process -Name 'device-agent', 'local-daemon', 'rdc', 'mcp-gateway', 'rdc-monitor' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500

        $agentProcess = Start-Process -FilePath $taskAgent -ArgumentList @('--profile', ('"' + $profileBase + '"'), 'run') -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 2

        if ($agentProcess.HasExited) {
            Write-WarnMessage "O agente não continuou rodando. Para ver o log detalhado, execute: & '$taskAgent' --profile '$profileBase' run"
        } else {
            Write-Success "Agente ativo em segundo plano (PID: $($agentProcess.Id))."
        }

        Write-Host "`n==========================================================" -ForegroundColor Green
        Write-Host "        INSTALAÇÃO CONCLUÍDA COM SUCESSO!                 " -ForegroundColor White
        Write-Host "==========================================================" -ForegroundColor Green
        Write-Host "Seu computador agora está conectado ao Remote Controller." -ForegroundColor White
        Write-Host "Você já pode fechar esta janela do PowerShell.`n" -ForegroundColor Cyan
        Write-Host "Comandos úteis:" -ForegroundColor Gray
        Write-Host "  Ver status:            & '$taskAgent' --profile '$profileBase' status" -ForegroundColor Gray
        Write-Host "  Parar agente:          & '$taskAgent' --profile '$profileBase' stop" -ForegroundColor Gray
        if (Test-Path -LiteralPath $taskMonitor) {
            Write-Host "  Monitor em tempo real: & '$taskMonitor'" -ForegroundColor Gray
        }
        Write-Host "  Desinstalar:           powershell -File '$uninstallScriptPath'" -ForegroundColor Gray
    } else {
        Write-Host "`n==========================================================" -ForegroundColor Yellow
        Write-Host "   INSTALAÇÃO PARCIAL - PAREAMENTO PENDENTE               " -ForegroundColor Yellow
        Write-Host "==========================================================" -ForegroundColor Yellow
        Write-Host "Os binários foram instalados com sucesso em '$versionRoot'." -ForegroundColor White
        Write-Host "O agente em segundo plano NÃO foi iniciado pois o pareamento não foi concluído." -ForegroundColor Yellow
        Write-Host "Assim que concluir o pareamento no portal, inicie o agente com:" -ForegroundColor White
        Write-Host "  & '$taskAgent' --profile '$profileBase' run" -ForegroundColor Cyan
        Write-Host ""
        Write-Host "Comandos úteis:" -ForegroundColor Gray
        if ($manualPairCommand) {
            Write-Host "  Concluir pareamento:   $manualPairCommand" -ForegroundColor Gray
        }
        if (Test-Path -LiteralPath $taskMonitor) {
            Write-Host "  Monitor em tempo real: & '$taskMonitor'" -ForegroundColor Gray
        }
        Write-Host "  Desinstalar:           powershell -File '$uninstallScriptPath'" -ForegroundColor Gray
        Write-Host ""
    }

} finally {
    if (Test-Path -LiteralPath $tempStaging) {
        Remove-Item -LiteralPath $tempStaging -Recurse -Force -ErrorAction SilentlyContinue
    }
}
