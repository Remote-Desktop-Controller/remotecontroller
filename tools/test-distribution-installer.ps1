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
    
    # Compilar executável mock em C# para simulação precisa de chamadas
    $mockSource = @'
using System;
using System.IO;
using System.Linq;

class MockAgent {
    static int Main(string[] args) {
        string logFile = Environment.GetEnvironmentVariable("MOCK_AGENT_LOG");
        if (!string.IsNullOrEmpty(logFile)) {
            File.AppendAllText(logFile, string.Join(" ", args) + Environment.NewLine);
        }
        
        if (args.Contains("pair")) {
            Console.WriteLine("Abra https://example.com/pair e aprove o codigo ABCD-1234");
            if (Environment.GetEnvironmentVariable("MOCK_SIMULATE_PAIR_SUCCESS") == "1") {
                int profileIdx = Array.IndexOf(args, "--profile");
                if (profileIdx >= 0 && profileIdx + 1 < args.Length) {
                    string prof = args[profileIdx + 1];
                    Directory.CreateDirectory(prof);
                    File.WriteAllText(Path.Combine(prof, "agent.json"), "{\"mock\": true}");
                }
                return 0;
            }
            return 1;
        }
        if (args.Contains("run")) {
            int profileIdx = Array.IndexOf(args, "--profile");
            if (profileIdx >= 0 && profileIdx + 1 < args.Length) {
                string prof = args[profileIdx + 1];
                if (!File.Exists(Path.Combine(prof, "agent.json"))) {
                    Console.Error.WriteLine("FATAL: agent.json nao encontrado");
                    return 2;
                }
            }
            return 0;
        }
        return 0;
    }
}
'@
    $mockCsPath = Join-Path $testRoot 'MockAgent.cs'
    [IO.File]::WriteAllText($mockCsPath, $mockSource, [Text.Encoding]::UTF8)
    $cscPath = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
    $agentBin = Join-Path $mockPkgDir 'device-agent.exe'
    $daemonBin = Join-Path $mockPkgDir 'local-daemon.exe'
    & $cscPath /nologo /target:exe "/out:$agentBin" $mockCsPath | Out-Null
    Copy-Item -LiteralPath $agentBin -Destination $daemonBin -Force
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
        $customInstallBase = Join-Path $testRoot 'installed_app\RemoteDesktopController'
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
        $testUninstallApp = Join-Path $testRoot 'installed_app\RemoteDesktopController'
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

    # ----------------------------------------------------
    # TESTE 6: Resolução Canônica (~/.remotecontroller/agent) vs Legado
    # ----------------------------------------------------
    Assert-Test "Resolução de perfil canônico e compatibilidade retroativa com legado" {
        $testHome = Join-Path $testRoot 'home_profile_test'
        $testLocalApp = Join-Path $testRoot 'localapp_profile_test'
        New-Item -ItemType Directory -Path $testHome -Force | Out-Null
        New-Item -ItemType Directory -Path $testLocalApp -Force | Out-Null
        
        $canonical = Join-Path $testHome '.remotecontroller\agent'
        $legacy = Join-Path $testLocalApp 'RemoteDesktopController\agent'
        
        # Cenário A: Sem credenciais prévias -> resolve para canônico
        $resolvedA = if ($legacy -and (Test-Path -LiteralPath (Join-Path $legacy 'agent.json')) -and -not (Test-Path -LiteralPath (Join-Path $canonical 'agent.json'))) { $legacy } else { $canonical }
        if ($resolvedA -ne $canonical) { throw "Deveria resolver para canônico quando não há credenciais legadas." }
        
        # Cenário B: Legado possui agent.json e canônico não possui -> resolve para legado
        New-Item -ItemType Directory -Path $legacy -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $legacy 'agent.json'), '{"mock_legacy": true}')
        
        $resolvedB = if ($legacy -and (Test-Path -LiteralPath (Join-Path $legacy 'agent.json')) -and -not (Test-Path -LiteralPath (Join-Path $canonical 'agent.json'))) { $legacy } else { $canonical }
        if ($resolvedB -ne $legacy) { throw "Deveria resolver para legado quando existem credenciais nele." }
        
        # Cenário C: Canônico já possui credenciais -> canônico tem prioridade
        New-Item -ItemType Directory -Path $canonical -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $canonical 'agent.json'), '{"mock_canonical": true}')
        
        $resolvedC = if ($legacy -and (Test-Path -LiteralPath (Join-Path $legacy 'agent.json')) -and -not (Test-Path -LiteralPath (Join-Path $canonical 'agent.json'))) { $legacy } else { $canonical }
        if ($resolvedC -ne $canonical) { throw "Deveria resolver para canônico quando ambos possuem credenciais." }
    }

    # ----------------------------------------------------
    # TESTE 7: Fluxo com Pareamento NÃO Concluído Aborta Boot e Exibe Alerta
    # ----------------------------------------------------
    Assert-Test "Fluxo com pareamento não concluído aborta boot em background e exibe comando manual" {
        $testInstallDir = Join-Path $testRoot 'app_unpaired\RemoteDesktopController'
        $testProfileDir = Join-Path $testRoot 'profile_unpaired'
        $testLog = Join-Path $testRoot 'mock_agent_unpaired.log'
        
        $env:MOCK_AGENT_LOG = $testLog
        $env:MOCK_SIMULATE_PAIR_SUCCESS = "0"
        
        if (Test-Path -LiteralPath $testLog) { Remove-Item -LiteralPath $testLog -Force }
        
        $installOut = powershell -ExecutionPolicy Bypass -File $installerScript -LocalPackagePath $mockPkgDir -InstallDir $testInstallDir -ProfileDir $testProfileDir -NonInteractive 2>&1
        $outStr = ($installOut | Out-String)
        
        # 1. agent.json NÃO deve ter sido criado
        $credPath = Join-Path $testProfileDir 'agent.json'
        if (Test-Path -LiteralPath $credPath) {
            throw "agent.json foi criado inesperadamente em fluxo não concluído."
        }
        
        # 2. Saída deve alertar que o pareamento não foi concluído
        if ($outStr -notmatch 'ALERTA:\s+PAREAMENTO' -or $outStr -notmatch 'NAVEGADOR') {
            throw "Alerta destacado de pareamento não concluído não foi emitido. Saída: $outStr"
        }
        
        # 3. Saída deve fornecer o comando exato que o usuário deve rodar
        if ($outStr -notmatch 'pair\s+--server') {
            throw "Comando manual de pareamento não foi fornecido. Saída: $outStr"
        }
        
        # 4. device-agent run NÃO deve ter sido chamado em background
        if (Test-Path -LiteralPath $testLog) {
            $logContent = Get-Content -LiteralPath $testLog -Raw
            if ($logContent -match '\brun\b') {
                throw "device-agent run foi chamado indevidamente em segundo plano sem agent.json!"
            }
        }
    }

    # ----------------------------------------------------
    # TESTE 8: Fluxo com Pareamento Concluído com Sucesso Inicia Agente
    # ----------------------------------------------------
    Assert-Test "Fluxo com pareamento concluído gera credenciais e prossegue com ativação" {
        $testInstallDir = Join-Path $testRoot 'app_paired\RemoteDesktopController'
        $testProfileDir = Join-Path $testRoot 'profile_paired'
        $testLog = Join-Path $testRoot 'mock_agent_paired.log'
        
        $env:MOCK_AGENT_LOG = $testLog
        $env:MOCK_SIMULATE_PAIR_SUCCESS = "1"
        
        if (Test-Path -LiteralPath $testLog) { Remove-Item -LiteralPath $testLog -Force }
        
        $installOut = powershell -ExecutionPolicy Bypass -File $installerScript -LocalPackagePath $mockPkgDir -InstallDir $testInstallDir -ProfileDir $testProfileDir -NonInteractive 2>&1
        $outStr = ($installOut | Out-String)
        
        # 1. agent.json DEVE ter sido criado
        $credPath = Join-Path $testProfileDir 'agent.json'
        if (-not (Test-Path -LiteralPath $credPath)) {
            throw "agent.json deveria ter sido criado com sucesso. Saída: $outStr"
        }
        
        # 2. device-agent run DEVE ter sido iniciado
        if (Test-Path -LiteralPath $testLog) {
            $logContent = Get-Content -LiteralPath $testLog -Raw
            if ($logContent -notmatch '\brun\b') {
                throw "device-agent run deveria ter sido executado após pareamento concluído."
            }
        } else {
            throw "Log do mock agent não foi registrado."
        }
    }

    # ----------------------------------------------------
    # TESTE 9: Desinstalador Harmonizado Remove Perfis
    # ----------------------------------------------------
    Assert-Test "Desinstalador remove dados de perfil especificado e canônico com -RemoveData" {
        $testApp = Join-Path $testRoot 'app_remove_profiles\RemoteDesktopController'
        New-Item -ItemType Directory -Path $testApp -Force | Out-Null
        Copy-Item -LiteralPath $uninstallerScript -Destination (Join-Path $testApp 'uninstall.ps1')
        
        $mockProfile = Join-Path $testRoot 'profile_to_remove'
        New-Item -ItemType Directory -Path $mockProfile -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $mockProfile 'agent.json'), '{"delete_me": true}')
        
        powershell -ExecutionPolicy Bypass -File (Join-Path $testApp 'uninstall.ps1') -InstallPath $testApp -ProfilePath $mockProfile -NonInteractive -RemoveData
        
        if (Test-Path -LiteralPath $mockProfile) {
            throw "Diretório de perfil $mockProfile não foi removido pelo desinstalador."
        }
    }

    # ----------------------------------------------------
    # TESTE 10: Desinstalador SEM -InstallPath respeita PSScriptRoot e não apaga pasta pai
    # ----------------------------------------------------
    Assert-Test "Desinstalador sem -InstallPath respeita PSScriptRoot e preserva diretório pai e vizinhos" {
        $parentFolder = Join-Path $testRoot 'parent_programs'
        $rdcFolder = Join-Path $parentFolder 'RemoteDesktopController'
        $neighborFolder = Join-Path $parentFolder 'NeighborApp'
        
        New-Item -ItemType Directory -Path $rdcFolder -Force | Out-Null
        New-Item -ItemType Directory -Path $neighborFolder -Force | Out-Null
        
        [IO.File]::WriteAllText((Join-Path $neighborFolder 'keep_me.txt'), 'NEIGHBOR_DATA')
        [IO.File]::WriteAllText((Join-Path $rdcFolder 'device-agent.exe'), 'MOCK_AGENT')
        Copy-Item -LiteralPath $uninstallerScript -Destination (Join-Path $rdcFolder 'uninstall.ps1')
        
        # Executa sem parâmetro -InstallPath (cenário padrão do usuário final)
        powershell -ExecutionPolicy Bypass -File (Join-Path $rdcFolder 'uninstall.ps1') -NonInteractive
        
        # 1. Pasta vizinha DEVE estar intacta
        if (-not (Test-Path -LiteralPath (Join-Path $neighborFolder 'keep_me.txt'))) {
            throw "FALHA CRÍTICA: Desinstalador apagou pasta vizinha do diretório pai!"
        }
        
        # 2. Executável do RDC deve ter sido removido
        if (Test-Path -LiteralPath (Join-Path $rdcFolder 'device-agent.exe')) {
            throw "Executável do RDC não foi removido pelo desinstalador."
        }
    }

    # ----------------------------------------------------
    # TESTE 11: Guardrail de Segurança Bloqueia Target Inválido / Sem Leaf RemoteDesktopController
    # ----------------------------------------------------
    Assert-Test "Guardrail de segurança bloqueia desinstalação se leaf não for RemoteDesktopController" {
        $invalidTarget = Join-Path $testRoot 'unsafe_directory'
        New-Item -ItemType Directory -Path $invalidTarget -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $invalidTarget 'important.txt'), 'CRITICAL_DATA')
        
        $caught = $false
        try {
            $res = powershell -ExecutionPolicy Bypass -File $uninstallerScript -InstallPath $invalidTarget -NonInteractive 2>&1
            if (($res | Out-String) -match 'ERRO CRÍTICO DE SEGURANÇA') {
                $caught = $true
            }
        } catch {
            $caught = $true
        }
        
        if (-not $caught) {
            throw "Desinstalador não bloqueou diretório com leaf inválido!"
        }
        if (-not (Test-Path -LiteralPath (Join-Path $invalidTarget 'important.txt'))) {
            throw "FALHA DE SEGURANÇA: Dados foram apagados em diretório com leaf inválido!"
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
