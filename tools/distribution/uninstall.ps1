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
Write-Host "        Remote Controller — Desinstalador Windows         " -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Cyan

$installRoot = if ($InstallPath) { $InstallPath } else { Join-Path $env:LOCALAPPDATA 'Programs\RemoteDesktopController' }
$profileRoot = if ($ProfilePath) { $ProfilePath } else { Join-Path $env:LOCALAPPDATA 'RemoteDesktopController\agent' }

# 1. Parar agentes e daemons ativos
Write-Host "`n==> Encerrando processos em execução..." -ForegroundColor Gray
Get-Process -Name 'device-agent', 'local-daemon' -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "  Encerrando $($_.Name) (PID $($_.Id))..." -ForegroundColor Yellow
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 1

# 2. Remover chave de inicialização automática (HKCU Run)
Write-Host "==> Removendo inicialização automática..." -ForegroundColor Gray
try {
    $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
    if (Test-Path $runKey) {
        $props = Get-ItemProperty -Path $runKey -ErrorAction SilentlyContinue
        if ($props) {
            $props.PSObject.Properties | Where-Object { $_.Name -like 'remotecontroller-*' } | ForEach-Object {
                Remove-ItemProperty -Path $runKey -Name $_.Name -Force -ErrorAction SilentlyContinue
                Write-Host "  Chave $($_.Name) removida do registro." -ForegroundColor Green
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
    if (Test-Path -LiteralPath $profileRoot) {
        Remove-Item -LiteralPath $profileRoot -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host "  Perfil e credenciais locais removidos com sucesso." -ForegroundColor Green
    }
} else {
    Write-Host "  Perfil local preservado em: $profileRoot" -ForegroundColor Gray
}

# 4. Remover executáveis e pastas instaladas
Write-Host "==> Removendo executáveis instalados..." -ForegroundColor Gray
if (Test-Path -LiteralPath $installRoot) {
    Get-ChildItem -Path $installRoot -ErrorAction SilentlyContinue | ForEach-Object {
        try {
            Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
        } catch {}
    }
    try {
        Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
    } catch {}
}

Write-Host ""
Write-Host "[OK] Remote Controller foi desinstalado do seu computador com sucesso." -ForegroundColor Green
Write-Host ""
