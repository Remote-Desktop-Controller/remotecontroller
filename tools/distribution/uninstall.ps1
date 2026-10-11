#requires -Version 5.1
<#
.SYNOPSIS
    Script de desinstalação segura do Remote Controller Windows.
    Encerra processos ativos, remove autostart e limpa binários e dados locais.
#>
[CmdletBinding()]
param(
    [string]$InstallPath,
    [string]$ProfilePath,
    [switch]$RemoveData,
    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "        Remote Controller - Desinstalador Windows         " -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Cyan

$canonicalInstall = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController')).TrimEnd('\', '/')
$installRoot = if ($InstallPath) {
    $InstallPath
} elseif (-not [string]::IsNullOrWhiteSpace($PSScriptRoot) -and (Split-Path -Path $PSScriptRoot -Leaf) -eq 'RemoteDesktopController') {
    $PSScriptRoot
} else {
    $canonicalInstall
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

# 1. Parar agentes e daemons ativos
Write-Host "`n==> Encerrando processos em execução..." -ForegroundColor Gray
$isCanonical = ($installRoot -eq $canonicalInstall)
Get-Process -Name 'device-agent', 'local-daemon', 'rdc', 'mcp-gateway', 'rdc-monitor' -ErrorAction SilentlyContinue | ForEach-Object {
    $shouldStop = $isCanonical
    if (-not $shouldStop) {
        try {
            if ($_.Path -and $_.Path -like "*$installRoot*") {
                $shouldStop = $true
            }
        } catch {}
    }
    if ($shouldStop) {
        Write-Host "  Encerrando $($_.Name) (PID $($_.Id))..." -ForegroundColor Yellow
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
}
Start-Sleep -Seconds 1

# 2. Remover chave de inicialização automática (HKCU Run)
Write-Host "==> Removendo inicialização automática..." -ForegroundColor Gray
try {
    $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
    if (Test-Path $runKey) {
        $props = Get-ItemProperty -Path $runKey -ErrorAction SilentlyContinue
        if ($props) {
            $isCanonical = ($installRoot -eq $canonicalInstall)
            $props.PSObject.Properties | Where-Object { $_.Name -like 'remotecontroller-*' } | ForEach-Object {
                $shouldRemove = $isCanonical
                if (-not $shouldRemove) {
                    $entryVal = [string]$_.Value
                    if ($entryVal -like "*$installRoot*") {
                        $shouldRemove = $true
                    } elseif ($entryVal -match '-EncodedCommand\s+([A-Za-z0-9+/=]+)') {
                        try {
                            $decoded = [System.Text.Encoding]::Unicode.GetString([System.Convert]::FromBase64String($Matches[1]))
                            if ($decoded -like "*$installRoot*") {
                                $shouldRemove = $true
                            }
                        } catch {}
                    }
                }
                if ($shouldRemove) {
                    Remove-ItemProperty -Path $runKey -Name $_.Name -Force -ErrorAction SilentlyContinue
                    Write-Host "  Chave $($_.Name) removida do registro." -ForegroundColor Green
                }
            }
        }
    }
} catch {
    Write-Host "  Aviso: Não foi possível verificar chaves de registro ($($_.Exception.Message))" -ForegroundColor Gray
}

# 3. Tratamento de dados locais e pareamento
if (-not $RemoveData -and -not $NonInteractive) {
    Write-Host ""
    $resp = Read-Host "Deseja remover as credenciais e configurações de pareamento locais? (S/N) [Padrão: N]"
    if ($resp -match '^(s|sim|y|yes)$') {
        $RemoveData = $true
    }
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
            Write-Host "  Perfil e credenciais locais removidos com sucesso: $p" -ForegroundColor Green
        }
    }
} else {
    Write-Host "  Perfil local preservado em: $profileRoot" -ForegroundColor Gray
}

# 4. Remover executáveis e pastas instaladas
Write-Host "==> Removendo executáveis instalados..." -ForegroundColor Gray
if (Test-Path -LiteralPath $installRoot) {
    Get-ChildItem -Path $installRoot -Exclude 'uninstall.ps1' -ErrorAction SilentlyContinue | ForEach-Object {
        try {
            Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
        } catch {}
    }
    try {
        Remove-Item -LiteralPath (Join-Path $installRoot 'uninstall.ps1') -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
    } catch {}
}

Write-Host ""
Write-Host "[OK] Remote Controller foi desinstalado do seu computador com sucesso." -ForegroundColor Green
Write-Host ""
