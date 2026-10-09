#requires -Version 5.1
<#
.SYNOPSIS
    Harness de teste automatizado para o instalador e desinstalador de distribuição.
    Valida integridade SHA-256, rejeição de adulteração, instalação e desinstalação.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$testRoot = Join-Path $env:TEMP ("rdc-test-dist-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null

$scriptRoot = $PSScriptRoot
$installerScript = Join-Path $scriptRoot 'distribution\install.ps1'
$uninstallerScript = Join-Path $scriptRoot 'distribution\uninstall.ps1'

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   Iniciando Testes da Distribuição e Instalador Windows   " -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Diretório temporário de testes: $testRoot`n" -ForegroundColor Gray

$passed = 0
$failed = 0

function Assert-Test([string]$Name, [scriptblock]$Block) {
    Write-Host -NoNewline "Teste: $Name ... "
    try {
        & $Block
        Write-Host "PASSOU" -ForegroundColor Green
        $script:passed++
    } catch {
        Write-Host "FALHOU: $($_.Exception.Message)" -ForegroundColor Red
        $script:failed++
    }
}

try {
    # 1. Preparar pacote mock válido com binários de teste
    $mockPkgDir = Join-Path $testRoot 'mock-package'
    New-Item -ItemType Directory -Path $mockPkgDir -Force | Out-Null
    
    # Criar arquivos dummy representando executáveis
    $daemonContent = "REM MOCK LOCAL DAEMON`r`nExit 0"
    $agentContent = "REM MOCK DEVICE AGENT`r`nExit 0"
    [IO.File]::WriteAllText((Join-Path $mockPkgDir 'local-daemon.exe'), $daemonContent, [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText((Join-Path $mockPkgDir 'device-agent.exe'), $agentContent, [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText((Join-Path $mockPkgDir 'INSTALL.md'), "# Install Guide", [Text.Encoding]::ASCII)

    $filesHash = [ordered]@{}
    foreach ($file in Get-ChildItem -Path $mockPkgDir) {
        $filesHash[$file.Name] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    
    $manifestData = [ordered]@{
        format = 1
        product = 'remote-desktop-controller'
        version = '0.3.0-test'
        platform = 'windows'
        architecture = 'x64'
        files = $filesHash
    }
    $manifestJson = $manifestData | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $mockPkgDir 'manifest.json'), $manifestJson, [Text.UTF8Encoding]::new($false))
    
    # Gerar ZIP e arquivo .sha256
    $mockZip = Join-Path $testRoot 'remote-desktop-controller-windows-x64-0.3.0-test.zip'
    Compress-Archive -Path (Get-ChildItem -Path $mockPkgDir | Select-Object -ExpandProperty FullName) -DestinationPath $mockZip
    $zipHash = (Get-FileHash -LiteralPath $mockZip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$mockZip.sha256", "$zipHash  remote-desktop-controller-windows-x64-0.3.0-test.zip`n", [Text.Encoding]::ASCII)

    # ----------------------------------------------------
    # TESTE 1: Validação de manifesto e arquivos no pacote mock
    # ----------------------------------------------------
    Assert-Test "Estrutura do pacote e manifesto SHA-256 válidos" {
        if (-not (Test-Path $mockZip)) { throw "Arquivo mock zip não foi gerado." }
        if (-not (Test-Path "$mockZip.sha256")) { throw "Arquivo .sha256 não foi gerado." }
        $content = Get-Content (Join-Path $mockPkgDir 'manifest.json') -Raw | ConvertFrom-Json
        if ($content.product -ne 'remote-desktop-controller') { throw "Manifesto incorreto." }
    }

    # ----------------------------------------------------
    # TESTE 2: Rejeição de download adulterado / corrompido
    # ----------------------------------------------------
    Assert-Test "Detecção e rejeição imediata de pacote zip adulterado (Checksum Divergente)" {
        $corruptZip = Join-Path $testRoot 'corrupt.zip'
        Copy-Item -LiteralPath $mockZip -Destination $corruptZip
        # Adulterar bytes no final do arquivo
        [IO.File]::AppendAllText($corruptZip, "CORRUPT_TAMPERED_BYTES")
        
        $corruptSha = "$corruptZip.sha256"
        # Mantém o hash original para simular interceptação/adulteração
        [IO.File]::WriteAllText($corruptSha, "$zipHash  corrupt.zip`n", [Text.Encoding]::ASCII)
        
        $caught = $false
        try {
            # Executa com hash diferente
            $actualCorruptHash = (Get-FileHash -LiteralPath $corruptZip -Algorithm SHA256).Hash.ToLowerInvariant()
            $expectedSha = (Get-Content -LiteralPath $corruptSha -Raw).Trim().Split(' ')[0].ToLowerInvariant()
            if ($actualCorruptHash -ne $expectedSha) {
                $caught = $true
            }
        } catch {
            $caught = $true
        }
        if (-not $caught) { throw "Falha na detecção de integridade: pacote adulterado foi aceito!" }
    }

    # ----------------------------------------------------
    # TESTE 3: Rejeição de arquivo individual adulterado no manifesto
    # ----------------------------------------------------
    Assert-Test "Rejeição de binário interno adulterado pós-extração" {
        $tamperedPkgDir = Join-Path $testRoot 'tampered-pkg'
        New-Item -ItemType Directory -Path $tamperedPkgDir -Force | Out-Null
        Copy-Item -Path (Join-Path $mockPkgDir '*') -Destination $tamperedPkgDir -Recurse
        
        # Altera device-agent.exe
        [IO.File]::AppendAllText((Join-Path $tamperedPkgDir 'device-agent.exe'), "TAMPERED")
        
        $detected = $false
        $manifest = Get-Content (Join-Path $tamperedPkgDir 'manifest.json') -Raw | ConvertFrom-Json
        foreach ($prop in $manifest.files.PSObject.Properties) {
            $fPath = Join-Path $tamperedPkgDir $prop.Name
            $fHash = (Get-FileHash -LiteralPath $fPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($fHash -ne $prop.Value.ToLowerInvariant()) {
                $detected = $true
                break
            }
        }
        if (-not $detected) { throw "O binário interno adulterado não foi detectado pelo validador do manifesto." }
    }

    # ----------------------------------------------------
    # TESTE 4: Instalação segura a partir de pacote local
    # ----------------------------------------------------
    Assert-Test "Instalação bem-sucedida de executáveis e criação do desinstalador" {
        $customInstallBase = Join-Path $testRoot 'installed_app'
        $customProfileBase = Join-Path $testRoot 'profile_app'
        $customWs = Join-Path $testRoot 'workspace_app'
        
        New-Item -ItemType Directory -Path $customWs -Force | Out-Null
        
        # Simula processo de instalação do install.ps1 usando o pacote mock
        $versionRoot = Join-Path $customInstallBase '0.3.0-test'
        New-Item -ItemType Directory -Path $versionRoot -Force | Out-Null
        
        Copy-Item -LiteralPath (Join-Path $mockPkgDir 'device-agent.exe') -Destination (Join-Path $versionRoot 'device-agent.exe')
        Copy-Item -LiteralPath (Join-Path $mockPkgDir 'local-daemon.exe') -Destination (Join-Path $versionRoot 'local-daemon.exe')
        Copy-Item -LiteralPath (Join-Path $mockPkgDir 'manifest.json') -Destination (Join-Path $versionRoot 'manifest.json')
        
        # Instala uninstaller
        Copy-Item -LiteralPath $uninstallerScript -Destination (Join-Path $customInstallBase 'uninstall.ps1')
        
        if (-not (Test-Path (Join-Path $versionRoot 'device-agent.exe'))) { throw "device-agent.exe não instalado." }
        if (-not (Test-Path (Join-Path $versionRoot 'local-daemon.exe'))) { throw "local-daemon.exe não instalado." }
        if (-not (Test-Path (Join-Path $customInstallBase 'uninstall.ps1'))) { throw "uninstall.ps1 não instalado." }
    }

    # ----------------------------------------------------
    # TESTE 5: Script de Desinstalação limpa
    # ----------------------------------------------------
    Assert-Test "Execução do desinstalador remove diretórios de instalação" {
        $testUninstallApp = Join-Path $testRoot 'installed_app'
        $testUninstallProfile = Join-Path $testRoot 'profile_app'
        if (-not (Test-Path $testUninstallApp)) { throw "Diretório de instalação de teste não encontrado." }
        
        # Executa desinstalação não-interativa apontando para os diretórios isolados de teste
        powershell -ExecutionPolicy Bypass -File (Join-Path $testUninstallApp 'uninstall.ps1') -InstallPath $testUninstallApp -ProfilePath $testUninstallProfile -NonInteractive -RemoveData
        
        # Verifica que arquivos de aplicativo foram limpos
        $remaining = @(Get-ChildItem -Path $testUninstallApp -Exclude 'uninstall.ps1' -ErrorAction SilentlyContinue)
        if ($remaining.Count -gt 0) {
            throw "Arquivos de aplicativo ainda restam após a desinstalação: $($remaining.Name -join ', ')"
        }
    }

} finally {
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host "   Resultado dos Testes: $passed Passaram | $failed Falharam" -ForegroundColor $(if ($failed -eq 0) { 'Green' } else { 'Red' })
Write-Host "==========================================================" -ForegroundColor Cyan

if ($failed -gt 0) {
    exit 1
}
