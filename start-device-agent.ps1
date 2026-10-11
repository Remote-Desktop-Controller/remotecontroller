#requires -Version 5.1
<#
.SYNOPSIS
    Script de Inicializacao e Controle Operacional do Remote Desktop Controller (device-agent).
    Gerencia execucao em background/foreground, status, parada e inicializacao automatica com Windows.
.EXAMPLE
    .\start-device-agent.ps1 -Background
.EXAMPLE
    .\start-device-agent.ps1 -Status
.EXAMPLE
    .\start-device-agent.ps1 -Stop
.EXAMPLE
    .\start-device-agent.ps1 -Autostart enable
#>
[CmdletBinding()]
param(
    [switch]$Background,
    [switch]$Foreground,
    [switch]$Status,
    [switch]$Stop,
    [ValidateSet('enable', 'disable', 'status')]
    [string]$Autostart,
    [string]$ProfilePath,
    [string]$BinaryPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-Info([string]$msg) {
    Write-Host "[*] $msg" -ForegroundColor Cyan
}

function Write-Success([string]$msg) {
    Write-Host "[+] $msg" -ForegroundColor Green
}

function Write-WarnMsg([string]$msg) {
    Write-Host "[!] $msg" -ForegroundColor Yellow
}

function Write-Err([string]$msg) {
    Write-Host "[-] $msg" -ForegroundColor Red
}

# 1. Localizacao do Binario do Agente
$agentExe = if ($BinaryPath -and (Test-Path -LiteralPath $BinaryPath)) {
    (Resolve-Path -LiteralPath $BinaryPath).Path
} else {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController\current\device-agent.exe'),
        'C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\target\release\device-agent.exe',
        'C:\Users\Notebook\Documents\Remote Desktop Controller\target\release\device-agent.exe',
        'C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\target\debug\device-agent.exe'
    )
    $found = $null
    foreach ($cand in $candidates) {
        if (Test-Path -LiteralPath $cand) {
            $found = $cand
            break
        }
    }
    $found
}

if (-not $agentExe) {
    Write-Err "device-agent.exe nao encontrado nos caminhos conhecidos."
    exit 1
}

# 2. Localizacao do Perfil de Credenciais
$profileArg = if ($ProfilePath -and (Test-Path -LiteralPath $ProfilePath)) {
    (Resolve-Path -LiteralPath $ProfilePath).Path
} else {
    $userHome = if ($HOME) { $HOME } else { $env:USERPROFILE }
    $pCandidates = @(
        (Join-Path $env:LOCALAPPDATA 'RemoteDesktopController\agent\agent.json'),
        (Join-Path $userHome '.remotecontroller\agent\agent.json')
    )
    $foundP = $null
    foreach ($cand in $pCandidates) {
        if (Test-Path -LiteralPath $cand) {
            $foundP = $cand
            break
        }
    }
    $foundP
}

# 3. Operacao: Parar Agente (-Stop)
if ($Stop) {
    Write-Info "Solicitando encerramento do device-agent..."
    try {
        if ($profileArg) {
            & $agentExe --profile $profileArg stop
        } else {
            & $agentExe stop
        }
        Write-Success "Sinal de parada enviado com sucesso."
    } catch {
        Write-WarnMsg "Tentando encerramento forcado de processos residuais..."
    }
    Get-Process -Name 'device-agent', 'local-daemon' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Write-Success "Processos do agente e daemon finalizados."
    exit 0
}

# 4. Operacao: Configuracao de Autostart (-Autostart enable/disable/status)
if ($Autostart) {
    if ($Autostart -eq 'status') {
        $runKey = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
        $matched = $false
        if ($runKey) {
            foreach ($prop in $runKey.PSObject.Properties) {
                if ($prop.Name -like 'remotecontroller*') {
                    Write-Success "Autostart ativo no Registro Windows: $($prop.Name)"
                    Write-Host "  Comando: $($prop.Value)" -ForegroundColor Gray
                    $matched = $true
                }
            }
        }
        if (-not $matched) {
            Write-WarnMsg "Nenhuma entrada de inicializacao automatica encontrada para o Remote Controller."
        }
    } elseif ($Autostart -eq 'enable') {
        Write-Info "Registrando inicializacao automatica com o Windows..."
        if ($profileArg) {
            & $agentExe --profile $profileArg autostart enable
        } else {
            & $agentExe autostart enable
        }
        Write-Success "Autostart configurado no Registro (HKCU\...\Run)."
    } elseif ($Autostart -eq 'disable') {
        Write-Info "Removendo inicializacao automatica com o Windows..."
        if ($profileArg) {
            & $agentExe --profile $profileArg autostart disable
        } else {
            & $agentExe autostart disable
        }
        Write-Success "Autostart desativado."
    }
    exit 0
}

# 5. Operacao: Status (-Status)
if ($Status) {
    Write-Info "Consultando status do device-agent..."
    Write-Host "Binario : $agentExe" -ForegroundColor Gray
    Write-Host "Perfil  : $profileArg" -ForegroundColor Gray
    if ($profileArg) {
        & $agentExe --profile $profileArg status
    } else {
        & $agentExe status
    }
    $runningAgent = Get-Process -Name 'device-agent' -ErrorAction SilentlyContinue
    $runningDaemon = Get-Process -Name 'local-daemon' -ErrorAction SilentlyContinue
    Write-Host "`nProcessos em Execucao:" -ForegroundColor Cyan
    Write-Host "  device-agent: $(if ($runningAgent) { 'Ativo (PID: ' + ($runningAgent.Id -join ', ') + ')' } else { 'Inativo' })"
    Write-Host "  local-daemon: $(if ($runningDaemon) { 'Ativo (PID: ' + ($runningDaemon.Id -join ', ') + ')' } else { 'Inativo' })"
    exit 0
}

# 6. Inicializacao do Agente (Background ou Foreground)
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "     Remote Desktop Controller - Inicializador do Agente    " -ForegroundColor White
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "Binario: $agentExe"
Write-Host "Perfil : $(if ($profileArg) { $profileArg } else { 'Auto-detectado pelo agente' })"

# Validar se o perfil existe antes de iniciar
if ($profileArg) {
    $statJson = & $agentExe --profile $profileArg status | ConvertFrom-Json -ErrorAction SilentlyContinue
    if ($statJson -and -not $statJson.paired) {
        Write-Err "Dispositivo nao esta pareado!"
        Write-Host $statJson.instructions -ForegroundColor Yellow
        exit 1
    }
    Write-Success "Dispositivo pareado com $($statJson.server) (Device ID: $($statJson.device_id))"
}

# Verificar se ja esta rodando
$alreadyRunning = Get-Process -Name 'device-agent' -ErrorAction SilentlyContinue
if ($alreadyRunning) {
    Write-WarnMsg "O device-agent ja esta em execucao (PID: $($alreadyRunning.Id -join ', '))."
    Write-Info "Use '.\start-device-agent.ps1 -Status' para verificar ou '.\start-device-agent.ps1 -Stop' para reiniciar."
    exit 0
}

$agentArgs = @()
if ($profileArg) {
    $agentArgs += @('--profile', $profileArg)
}
$agentArgs += 'run'

if ($Background) {
    Write-Info "Iniciando device-agent em segundo plano (Background/Hidden)..."
    $argList = if ($profileArg) {
        @('--profile', "`"$profileArg`"", 'run')
    } else {
        @('run')
    }
    $proc = Start-Process -FilePath $agentExe -ArgumentList $argList -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 3
    if (-not $proc.HasExited) {
        Write-Success "Agente iniciado com sucesso em segundo plano (PID: $($proc.Id))!"
        Write-Info "Enviando heartbeats a cada 15 segundos para a VPS."
        Write-Info "Use '.\start-device-agent.ps1 -Status' para monitorar ou '.\start-device-agent.ps1 -Stop' para encerrar."
    } else {
        Write-Err "Falha ao iniciar agente em segundo plano. Verifique logs."
        exit 1
    }
} else {
    Write-Info "Iniciando device-agent em primeiro plano (Pressione Ctrl+C para encerrar)..."
    & $agentExe @agentArgs
}
