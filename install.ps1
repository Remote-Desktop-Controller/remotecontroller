<#
.SYNOPSIS
    Instalador Canônico e Seguro do Remote Desktop Controller (RDC) no Windows.
.DESCRIPTION
    Instala os binários do Remote Desktop Controller no escopo de usuário ($env:LOCALAPPDATA\Programs\RemoteDesktopController),
    garante a separação física estrita entre binários e dados de usuário (~/.remotecontroller),
    implanta o desinstalador estático canônico 'uninstall.ps1' (sem geração dinâmica defeituosa)
    e configura as permissões de acesso privado no sistema de arquivos.
.PARAMETER SourceDir
    Diretório contendo os binários construídos (padrão: target/release ou pasta atual).
.PARAMETER InstallDir
    Diretório de destino dos binários. Padrão: $env:LOCALAPPDATA\Programs\RemoteDesktopController.
.PARAMETER DataDir
    Diretório de dados e estado. Padrão: $HOME\.remotecontroller.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$SourceDir = (Join-Path $PSScriptRoot "target\release"),

    [Parameter(Mandatory = $false)]
    [string]$InstallDir = (Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::LocalApplicationData)) "Programs\RemoteDesktopController"),

    [Parameter(Mandatory = $false)]
    [string]$DataDir = (Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::UserProfile)) ".remotecontroller")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Output "================================================================="
Write-Output "  REMOTE DESKTOP CONTROLLER (RDC) - INSTALADOR CANÔNICO v0.2.0   "
Write-Output "================================================================="

# 1. Normalização e Validação de Diretórios
$cleanInstallDir = [System.IO.Path]::GetFullPath($InstallDir).TrimEnd('\', '/')
$installLeaf = Split-Path -Path $cleanInstallDir -Leaf

if ($installLeaf -ne "RemoteDesktopController") {
    throw "ERRO DE SEGURANÇA: O diretório de instalação deve se chamar 'RemoteDesktopController'. Recebido: '$cleanInstallDir'."
}

$cleanDataDir = [System.IO.Path]::GetFullPath($DataDir).TrimEnd('\', '/')
$dataLeaf = Split-Path -Path $cleanDataDir -Leaf

if ($dataLeaf -ne ".remotecontroller") {
    throw "ERRO DE SEGURANÇA: O diretório de dados deve se chamar '.remotecontroller'. Recebido: '$cleanDataDir'."
}

Write-Output "[AGENT-SECOPS] Diretório de Binários: $cleanInstallDir"
Write-Output "[AGENT-STORAGE] Diretório de Dados:   $cleanDataDir"

# 2. Encerramento de instâncias prévias antes de atualizar arquivos
$agentProcs = @("rdc", "local-daemon", "mcp-gateway", "device-agent")
foreach ($p in $agentProcs) {
    $found = Get-Process -Name $p -ErrorAction SilentlyContinue
    if ($found) {
        Write-Output "[AGENT-SECOPS] Finalizando processo ativo: $p..."
        $found | Stop-Process -Force -ErrorAction SilentlyContinue
    }
}

# 3. Criação da árvore de binários do aplicativo
if (-not (Test-Path -LiteralPath $cleanInstallDir)) {
    New-Item -ItemType Directory -Path $cleanInstallDir -Force | Out-Null
}

# Copia os binários disponíveis se existirem na pasta de origem
$binariesToInstall = @("rdc.exe", "local-daemon.exe", "mcp-gateway.exe", "device-agent.exe", "manifest.json")
$installedCount = 0

foreach ($bin in $binariesToInstall) {
    $srcFile = Join-Path $SourceDir $bin
    if (Test-Path -LiteralPath $srcFile) {
        Copy-Item -LiteralPath $srcFile -Destination (Join-Path $cleanInstallDir $bin) -Force
        $installedCount++
        Write-Output "[AGENT-SECOPS] Binário instalado: $bin"
    }
}

if ($installedCount -eq 0) {
    Write-Warning "[AGENT-SECOPS] Nenhum binário pré-compilado encontrado em '$SourceDir'. Para compilar a suíte completa de binários, execute: cargo build --release."
}

# 4. Implantação do Desinstalador Estático Canônico (uninstall.ps1)
$canonicalUninstallerSrc = Join-Path $PSScriptRoot "uninstall.ps1"
$targetUninstaller = Join-Path $cleanInstallDir "uninstall.ps1"

if (Test-Path -LiteralPath $canonicalUninstallerSrc) {
    Copy-Item -LiteralPath $canonicalUninstallerSrc -Destination $targetUninstaller -Force
    Write-Output "[AGENT-SECOPS] Desinstalador canônico estático implantado em '$targetUninstaller'."
} else {
    throw "ERRO FATAL: Arquivo 'uninstall.ps1' canônico não encontrado em '$PSScriptRoot'."
}

# 5. Inicialização da Arquitetura de Persistência ~/.remotecontroller
Write-Output "[AGENT-STORAGE] Estruturando layout de persistência e isolamento em '$cleanDataDir'..."

$subdirectories = @("sessions", "checkpoints", "logs", ".vault", "agent")
foreach ($sub in $subdirectories) {
    $path = Join-Path $cleanDataDir $sub
    if (-not (Test-Path -LiteralPath $path)) {
        New-Item -ItemType Directory -Path $path -Force | Out-Null
    }
}

# 5.1. Verificação de compatibilidade retroativa para perfil de agente
$legacyAgentProfile = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "RemoteDesktopController\agent" } else { $null }
$canonicalAgentProfile = Join-Path $cleanDataDir "agent"
if ($legacyAgentProfile -and (Test-Path -LiteralPath (Join-Path $legacyAgentProfile "agent.json")) -and -not (Test-Path -LiteralPath (Join-Path $canonicalAgentProfile "agent.json"))) {
    Write-Output "[AGENT-STORAGE] Perfil legado com credenciais detectado em '$legacyAgentProfile'. Compatibilidade retroativa mantida."
}

# Criação do config.json inicial caso não exista
$configPath = Join-Path $cleanDataDir "config.json"
if (-not (Test-Path -LiteralPath $configPath)) {
    $defaultConfig = @{
        version = "0.2.0"
        created_at = (Get-Date).ToString("o")
        security = @{
            vault_storage = "dpapi"
            telemetry_pipe = "\\.\pipe\rdc-telemetry"
            audit_logging = $true
        }
    } | ConvertTo-Json -Depth 5

    [System.IO.File]::WriteAllText($configPath, $defaultConfig, [System.Text.UTF8Encoding]::new($false))
    Write-Output "[AGENT-STORAGE] Configuração padrão criada em '$configPath'."
}

# 6. Aplicação de ACLs NTFS restritivas para proteger dados do usuário
try {
    # icacls para garantir que apenas o usuário atual tem controle total (remove herança e usuários gerais)
    $aclArgs = "`"$cleanDataDir`" /inheritance:r /grant:r `"$($env:USERNAME):(OI)(CI)F`""
    $icaclsOut = cmd.exe /c "icacls $aclArgs" 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-Output "[AGENT-STORAGE] ACLs restritivas aplicadas em '$cleanDataDir' (Acesso exclusivo: $env:USERNAME)."
    } else {
        Write-Warning "Aviso: icacls retornou código $LASTEXITCODE. Permissões padrão mantidas."
    }
} catch {
    Write-Warning "Falha ao definir ACLs via icacls. Permissões de sistema de arquivos padrão mantidas."
}

# 7. Registro no Windows (Adicionar/Remover Programas do Painel de Controle)
try {
    $regKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\RemoteDesktopController"
    if (-not (Test-Path -Path $regKey)) {
        New-Item -Path $regKey -Force | Out-Null
    }

    Set-ItemProperty -Path $regKey -Name "DisplayName" -Value "Remote Desktop Controller (RDC)"
    Set-ItemProperty -Path $regKey -Name "DisplayVersion" -Value "0.2.0"
    Set-ItemProperty -Path $regKey -Name "Publisher" -Value "Remote Desktop Controller Systems"
    Set-ItemProperty -Path $regKey -Name "InstallLocation" -Value $cleanInstallDir
    Set-ItemProperty -Path $regKey -Name "UninstallString" -Value "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$targetUninstaller`""
    Set-ItemProperty -Path $regKey -Name "QuietUninstallString" -Value "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$targetUninstaller`" -Force"
    Set-ItemProperty -Path $regKey -Name "NoModify" -Value 1 -Type DWord
    Set-ItemProperty -Path $regKey -Name "NoRepair" -Value 1 -Type DWord
    Write-Output "[AGENT-SECOPS] Registro de desinstalação configurado com sucesso em HKCU."
} catch {
    Write-Warning "Não foi possível registrar no Painel de Controle do Windows."
}

Write-Output "================================================================="
Write-Output "  INSTALAÇÃO CONCLUÍDA COM SUCESSO! RDC PRONTO PARA OPERAÇÃO     "
Write-Output "================================================================="
