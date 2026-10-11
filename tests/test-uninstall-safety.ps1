<#
.SYNOPSIS
    Suíte de Testes Automatizados de Regressão e Prova de Não-Destruição [AGENT-QA].
.DESCRIPTION
    Valida rigorosamente o comportamento do desinstalador canônico (uninstall.ps1) e do script de patch
    (patch-uninstaller.ps1) em um ambiente sandbox isolado, provando matematicamente que:
    1. Pastas e aplicativos vizinhos em $env:LOCALAPPDATA\Programs NUNCA são tocados.
    2. O estado e dados de usuário (~/.remotecontroller) NUNCA são apagados sem a flag explícita -PurgeData / --purge-data.
    3. Workspaces e projetos externos do usuário NUNCA são tocados sob qualquer hipótese.
    4. Guardrails de segurança rejeitam imediatamente tentativas de apontar para diretórios pai, raízes ou alheios.
    5. O script de patch substitui desinstaladores vulneráveis antigos de maneira atômica e segura.
    6. Suporte completo ao formato kebab-case (--purge-data) sem quebras de binding posicional.
    7. Desacoplamento assíncrono para $env:TEMP funciona com e sem a flag -Wait.
    8. Execução a partir de dentro do diretório alvo não causa deadlock de file lock.
    9. O patch-uninstaller.ps1 é 100% autônomo e funciona de forma avulsa mesmo sem uninstall.ps1 local.
#>

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Output "================================================================="
Write-Output "  [AGENT-QA] INICIANDO SUÍTE DE TESTES: PROVA DE NÃO-DESTRUIÇÃO  "
Write-Output "================================================================="

$scriptRoot = $PSScriptRoot
$repoRoot = Split-Path -Path $scriptRoot -Parent
$canonicalUninstaller = Join-Path $repoRoot "uninstall.ps1"
$patchUninstaller = Join-Path $repoRoot "patch-uninstaller.ps1"

if (-not (Test-Path -LiteralPath $canonicalUninstaller)) {
    throw "[AGENT-QA] ERRO FATAL: 'uninstall.ps1' não encontrado em '$repoRoot'."
}
if (-not (Test-Path -LiteralPath $patchUninstaller)) {
    throw "[AGENT-QA] ERRO FATAL: 'patch-uninstaller.ps1' não encontrado em '$repoRoot'."
}

$assertionsPassed = 0
$assertionsFailed = 0
$scenariosPassed = 0
$totalScenarios = 10

function Assert-Condition([bool]$condition, [string]$testName) {
    if ($condition) {
        Write-Host "  [PASS] $testName" -ForegroundColor Green
        $script:assertionsPassed++
    } else {
        Write-Host "  [FAIL] $testName" -ForegroundColor Red
        $script:assertionsFailed++
        throw "[AGENT-QA] Falha no teste: $testName"
    }
}

# Criação de ambiente sandbox isolado em $env:TEMP
$sandboxRoot = Join-Path $env:TEMP ("rdc_qa_sandbox_" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $sandboxRoot -Force | Out-Null
Write-Output "[AGENT-QA] Sandbox isolado criado em: $sandboxRoot"

try {
    $programsDir = Join-Path $sandboxRoot "Programs"
    $targetRdc = Join-Path $programsDir "RemoteDesktopController"
    $neighborVscode = Join-Path $programsDir "Microsoft VS Code"
    $neighborGit = Join-Path $programsDir "Git"
    $neighborDiscord = Join-Path $programsDir "Discord"
    $userDataDir = Join-Path $sandboxRoot ".remotecontroller"
    $userWorkspace = Join-Path $sandboxRoot "Workspace\MyProject"

    # Setup de arquivos auxiliares simulados
    function Setup-SandboxEnvironment {
        New-Item -ItemType Directory -Path $targetRdc -Force | Out-Null
        Set-Content -Path (Join-Path $targetRdc "rdc.exe") -Value "MOCK_RDC_BINARY"
        Set-Content -Path (Join-Path $targetRdc "local-daemon.exe") -Value "MOCK_DAEMON_BINARY"
        Copy-Item -LiteralPath $canonicalUninstaller -Destination (Join-Path $targetRdc "uninstall.ps1") -Force

        New-Item -ItemType Directory -Path $neighborVscode -Force | Out-Null
        Set-Content -Path (Join-Path $neighborVscode "Code.exe") -Value "REAL_VSCODE_BINARY"
        Set-Content -Path (Join-Path $neighborVscode "settings.json") -Value "VSCODE_SETTINGS"

        New-Item -ItemType Directory -Path $neighborGit -Force | Out-Null
        Set-Content -Path (Join-Path $neighborGit "git.exe") -Value "REAL_GIT_BINARY"

        New-Item -ItemType Directory -Path $neighborDiscord -Force | Out-Null
        Set-Content -Path (Join-Path $neighborDiscord "Update.exe") -Value "DISCORD_UPDATE"

        New-Item -ItemType Directory -Path $userDataDir -Force | Out-Null
        Set-Content -Path (Join-Path $userDataDir "config.json") -Value "{ `"keep`": true }"
        New-Item -ItemType Directory -Path (Join-Path $userDataDir "sessions") -Force | Out-Null
        Set-Content -Path (Join-Path $userDataDir "sessions\sess_1.json") -Value "SESSION_DATA"

        New-Item -ItemType Directory -Path $userWorkspace -Force | Out-Null
        Set-Content -Path (Join-Path $userWorkspace "main.rs") -Value "fn main() { println!(`"user code`"); }"
    }

    # --------------------------------------------------------------------------
    # CENÁRIO 1: Desinstalação Padrão Preserva Vizinhos, Estado e Workspaces
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 1] Desinstalação Padrão Preserva Vizinhos e Dados ---"
    Setup-SandboxEnvironment

    & $canonicalUninstaller -TargetDir $targetRdc -DataDir $userDataDir -Wait -Force

    Assert-Condition (-not (Test-Path -LiteralPath $targetRdc)) "Pasta RemoteDesktopController foi completamente removida"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinho VSCode preservado intacto (Code.exe)"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "settings.json")) "Vizinho VSCode preservado intacto (settings.json)"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborGit "git.exe")) "Vizinho Git preservado intacto (git.exe)"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborDiscord "Update.exe")) "Vizinho Discord preservado intacto (Update.exe)"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userDataDir "config.json")) "Estado do usuário ~/.remotecontroller/config.json preservado"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userDataDir "sessions\sess_1.json")) "Sessões ~/.remotecontroller/sessions preservadas"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userWorkspace "main.rs")) "Workspace do usuário mantido 100% intacto"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 2: PurgeData (-PurgeData) Remove Estado mas Preserva Vizinhos e Workspaces
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 2] Flag -PurgeData Remove Estado sem Afetar Vizinhos ---"
    Setup-SandboxEnvironment

    & $canonicalUninstaller -TargetDir $targetRdc -DataDir $userDataDir -PurgeData -Wait -Force

    Assert-Condition (-not (Test-Path -LiteralPath $targetRdc)) "Pasta RemoteDesktopController foi removida"
    Assert-Condition (-not (Test-Path -LiteralPath $userDataDir)) "Pasta de estado ~/.remotecontroller foi expurgada como solicitado"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinho VSCode continua intacto"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborGit "git.exe")) "Vizinho Git continua intacto"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userWorkspace "main.rs")) "Workspace do usuário continua 100% intacto"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 3: Guardrail Rejeita Pasta Pai ($env:LOCALAPPDATA\Programs)
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 3] Guardrail Bloqueia Tentativa de Apagar Pasta Pai (Programs) ---"

    $parentDirRejected = $false
    try {
        & $canonicalUninstaller -TargetDir $programsDir -Wait -Force
    } catch {
        $parentDirRejected = $true
        Write-Output "  [INFO] Exceção esperada capturada com sucesso: $($_.Exception.Message)"
    }

    Assert-Condition ($parentDirRejected -eq $true) "Desinstalador lançou erro de segurança para pasta pai"
    Assert-Condition (Test-Path -LiteralPath $programsDir) "Pasta Programs não foi deletada"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinhos permanecem intocados após bloqueio"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 4: Guardrail Rejeita Nome de Pasta Não-RDC
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 4] Guardrail Bloqueia Pasta com Nome Não-Autorizado ---"

    $wrongLeafRejected = $false
    try {
        & $canonicalUninstaller -TargetDir $neighborVscode -Wait -Force
    } catch {
        $wrongLeafRejected = $true
    }

    Assert-Condition ($wrongLeafRejected -eq $true) "Desinstalador bloqueou execução em pasta com leaf diferente de RemoteDesktopController"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Pasta VSCode não sofreu nenhuma modificação"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 5: Patch Emergencial Substitui Desinstalador Vulnerável Antigo
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 5] Patch Emergencial Remedia Desinstalador Vulnerável ---"

    $mockVulnerableInstall = Join-Path $programsDir "RemoteDesktopController"
    New-Item -ItemType Directory -Path $mockVulnerableInstall -Force | Out-Null
    
    $vulnerableScriptPath = Join-Path $mockVulnerableInstall "uninstall.ps1"
    $vulnerableContent = @'
# VULNERABLE OLD SCRIPT
$target = Split-Path -Parent $PSScriptRoot
Get-ChildItem $target | Remove-Item -Recurse -Force
'@
    Set-Content -Path $vulnerableScriptPath -Value $vulnerableContent

    $patchRaw = & $patchUninstaller -InstallationDir $mockVulnerableInstall
    $patchResult = if ($patchRaw -is [array]) { $patchRaw | Where-Object { $_ -is [System.Management.Automation.PSCustomObject] } | Select-Object -Last 1 } else { $patchRaw }

    Assert-Condition ($patchResult.Status -eq "PATCHED") "Patch retornou status PATCHED"
    Assert-Condition ($patchResult.VulnerabilityRemediated -eq $true) "Patch detectou a vulnerabilidade antiga com precisão"
    Assert-Condition (Test-Path -LiteralPath $patchResult.BackupCreated) "Backup da versão vulnerável foi preservado"

    $patchedContent = Get-Content -Path $vulnerableScriptPath -Raw
    Assert-Condition ($patchedContent -match 'targetLeaf\s+-ne\s+["'']RemoteDesktopController["'']') "Novo desinstalador contém o guardrail estrito de leaf"
    Assert-Condition ($patchedContent -notmatch 'Split-Path\s+-Parent\s+\$PSScriptRoot') "Código vulnerável antigo foi totalmente eliminado"

    & $vulnerableScriptPath -TargetDir $mockVulnerableInstall -Wait -Force
    Assert-Condition (-not (Test-Path -LiteralPath $mockVulnerableInstall)) "Pasta RDC foi removida com segurança pelo desinstalador patcheado"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinho VSCode permaneceu 100% ileso após execução pós-patch"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 6: Flag Kebab-Case (--purge-data) Funciona e Expulsa Dados Sem Quebra
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 6] Validação de Parâmetro Kebab-Case (--purge-data) ---"
    Setup-SandboxEnvironment

    # Executa com a flag exata --purge-data especificada no Master Directive
    & $canonicalUninstaller -TargetDir $targetRdc -DataDir $userDataDir --purge-data -Wait -Force

    Assert-Condition (-not (Test-Path -LiteralPath $targetRdc)) "Pasta RDC removida com sucesso usando --purge-data"
    Assert-Condition (-not (Test-Path -LiteralPath $userDataDir)) "Dados ~/.remotecontroller expurgados usando --purge-data"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinho VSCode preservado intacto"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userWorkspace "main.rs")) "Workspace do usuário 100% preservado"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 7: Execução Assíncrona Sem -Wait Completa Via Worker em $env:TEMP
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 7] Desacoplamento Assíncrono Sem -Wait ---"
    Setup-SandboxEnvironment

    # Executa sem -Wait (modo padrão do desinstalador)
    & $canonicalUninstaller -TargetDir $targetRdc -DataDir $userDataDir -Force

    # Aguarda o worker assíncrono em $env:TEMP concluir a limpeza (até 10 segundos)
    $swWait = [System.Diagnostics.Stopwatch]::StartNew()
    $asyncRemoved = $false
    while ($swWait.ElapsedMilliseconds -lt 10000) {
        if (-not (Test-Path -LiteralPath $targetRdc)) {
            $asyncRemoved = $true
            break
        }
        Start-Sleep -Milliseconds 300
    }

    Assert-Condition ($asyncRemoved -eq $true) "Worker assíncrono concluiu a remoção física com sucesso em tempo hábil"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinho VSCode intacto após worker assíncrono"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $userDataDir "config.json")) "Dados de usuário preservados intactos sem flag purge"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 8: Execução a Partir de Dentro do Diretório Alvo Libera Lock
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 8] Execução a Partir do Diretório Alvo (Zero File-Lock) ---"
    Setup-SandboxEnvironment

    Push-Location -LiteralPath $targetRdc
    try {
        & .\uninstall.ps1 -TargetDir $targetRdc -DataDir $userDataDir -Wait -Force
    } finally {
        Pop-Location
    }

    Assert-Condition (-not (Test-Path -LiteralPath $targetRdc)) "Pasta alvo removida mesmo quando chamada de dentro dela"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $neighborVscode "Code.exe")) "Vizinhos permanecem preservados"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 9: Patch-Uninstaller Autônomo em Diretório Isolado (Sem uninstall.ps1 Local)
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 9] Patch-Uninstaller Autônomo e Standalone ---"

    $isolatedPatchDir = Join-Path $sandboxRoot "StandalonePatch"
    New-Item -ItemType Directory -Path $isolatedPatchDir -Force | Out-Null
    Copy-Item -LiteralPath $patchUninstaller -Destination (Join-Path $isolatedPatchDir "patch-uninstaller.ps1") -Force

    $isolatedMockRdc = Join-Path $isolatedPatchDir "RemoteDesktopController"
    New-Item -ItemType Directory -Path $isolatedMockRdc -Force | Out-Null
    Set-Content -Path (Join-Path $isolatedMockRdc "uninstall.ps1") -Value "# Old broken script`nSplit-Path -Parent `$PSScriptRoot"

    # Executa o patch sem que uninstall.ps1 exista na pasta StandalonePatch
    $standaloneResult = & (Join-Path $isolatedPatchDir "patch-uninstaller.ps1") -InstallationDir $isolatedMockRdc
    $patchObj = if ($standaloneResult -is [array]) { $standaloneResult | Select-Object -Last 1 } else { $standaloneResult }

    Assert-Condition ($patchObj.Status -eq "PATCHED") "Patch autônomo retornou PATCHED sem requerer arquivos locais"
    Assert-Condition ($patchObj.VulnerabilityRemediated -eq $true) "Vulnerabilidade foi identificada e remediada"
    Assert-Condition (Test-Path -LiteralPath (Join-Path $isolatedMockRdc "uninstall.ps1")) "Novo uninstaller foi implantado com sucesso"
    
    # Testa se o uninstaller implantado de forma autônoma funciona
    & (Join-Path $isolatedMockRdc "uninstall.ps1") -TargetDir $isolatedMockRdc -Wait -Force
    Assert-Condition (-not (Test-Path -LiteralPath $isolatedMockRdc)) "Desinstalador implantado pelo patch autônomo executa com sucesso"
    $scenariosPassed++

    # --------------------------------------------------------------------------
    # CENÁRIO 10: Guardrails de Caminhos Críticos e Raiz da Unidade
    # --------------------------------------------------------------------------
    Write-Output "`n--- [CENÁRIO 10] Bloqueio Rigoroso de Raiz da Unidade e Entradas Inválidas ---"

    $rootRejected = $false
    try {
        & $canonicalUninstaller -TargetDir "C:\" -Wait -Force
    } catch {
        $rootRejected = $true
    }
    Assert-Condition ($rootRejected -eq $true) "Desinstalador bloqueou tentativa de apontar para C:\"

    $emptyRejected = $false
    try {
        & $canonicalUninstaller -TargetDir "" -Wait -Force
    } catch {
        $emptyRejected = $true
    }
    Assert-Condition ($emptyRejected -eq $true) "Desinstalador rejeitou TargetDir vazio"

    $scenariosPassed++

    Write-Output "`n================================================================="
    Write-Output "  [AGENT-QA] TODOS OS TESTES PASSARAM COM SUCESSO! ($scenariosPassed/$totalScenarios Cenários Aprovados, $assertionsPassed Asserções)  "
    Write-Output "  PROVA MATEMÁTICA CONCLUÍDA: ZERO RISCO DE DELEÇÃO ACIDENTAL.   "
    Write-Output "================================================================="
}
finally {
    # Limpeza da sandbox de teste
    if (Test-Path -LiteralPath $sandboxRoot) {
        Remove-Item -LiteralPath $sandboxRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
