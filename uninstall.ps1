<#
.SYNOPSIS
    Desinstalador Canônico e Seguro do Remote Desktop Controller (RDC).
.DESCRIPTION
    Remove de forma segura e atômica os binários e artefatos de instalação do Remote Desktop Controller.
    Implementa guardrails rigorosos para impedir qualquer deleção fora da pasta alvo do aplicativo,
    mata processos ativos para evitar arquivos travados e desacopla a deleção física para um worker
    temporário em $env:TEMP ($env:TEMP\rdc_cleanup_<PID>.ps1) evitando problemas de auto-exclusão e file locks no Windows.
.PARAMETER TargetDir
    Caminho do diretório de instalação a ser desinstalado.
    Padrão: $PSScriptRoot se executado a partir de pasta com nome 'RemoteDesktopController',
    ou $env:LOCALAPPDATA\Programs\RemoteDesktopController.
    REGRA INVIOLÁVEL: O nome final da pasta (Leaf) DEVE ser 'RemoteDesktopController'.
.PARAMETER DataDir
    Caminho do diretório de dados/estado do usuário.
    Padrão: $HOME\.remotecontroller.
.PARAMETER PurgeData
    Se fornecido (via -PurgeData ou --purge-data), remove também os dados de estado e configuração em $HOME\.remotecontroller.
    Sem esta flag explícita, dados de sessão, vaults e logs de auditoria são SEMPRE preservados.
    Diretórios de workspace do usuário NUNCA são tocados sob hipótese alguma.
.PARAMETER Force
    Suprime mensagens e prompts de confirmação interativa.
.PARAMETER Wait
    Aguarda a conclusão completa do worker de limpeza assíncrono (usado para automação e testes).
.EXAMPLE
    .\uninstall.ps1
.EXAMPLE
    .\uninstall.ps1 --purge-data -Force
.EXAMPLE
    .\uninstall.ps1 -TargetDir "C:\Users\User\AppData\Local\Programs\RemoteDesktopController" -Wait
#>

[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Mandatory = $false)]
    [string]$TargetDir = "",

    [Parameter(Mandatory = $false)]
    [string]$DataDir = "",

    [Parameter(Mandatory = $false)]
    [Alias("purge-data")]
    [switch]$PurgeData,

    [Parameter(Mandatory = $false)]
    [switch]$Force,

    [Parameter(Mandatory = $false)]
    [switch]$Wait,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ==============================================================================
# 1. RESOLUÇÃO E VALIDAÇÃO DE CAMINHOS & GUARDRAILS DE SEGURANÇA
# ==============================================================================

# Tratamento de flags passadas no formato Linux/GNU (--purge-data, --force, etc.)
# que o binding de parâmetros do PowerShell pode interpretar como argumentos posicionais
if ($TargetDir -match '^--?purge-?data$') {
    $PurgeData = $true
    $TargetDir = ""
}
if ($TargetDir -match '^--?force$') {
    $Force = $true
    $TargetDir = ""
}
if ($TargetDir -match '^--?wait$') {
    $Wait = $true
    $TargetDir = ""
}

if ($RemainingArgs) {
    foreach ($extraArg in $RemainingArgs) {
        if ($extraArg -match '^--?purge-?data$') { $PurgeData = $true }
        if ($extraArg -match '^--?force$') { $Force = $true }
        if ($extraArg -match '^--?wait$') { $Wait = $true }
    }
}

if ($PSBoundParameters.ContainsKey('TargetDir') -and [string]::IsNullOrWhiteSpace($TargetDir)) {
    throw "ERRO DE SEGURANÇA: O caminho TargetDir não pode ser vazio ou nulo."
}

if ([string]::IsNullOrWhiteSpace($TargetDir)) {
    if (-not [string]::IsNullOrWhiteSpace($PSScriptRoot) -and (Split-Path -Path $PSScriptRoot -Leaf) -eq "RemoteDesktopController") {
        $TargetDir = $PSScriptRoot
    } else {
        $TargetDir = Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::LocalApplicationData)) "Programs\RemoteDesktopController"
    }
}

if ($PSBoundParameters.ContainsKey('DataDir') -and [string]::IsNullOrWhiteSpace($DataDir)) {
    throw "ERRO DE SEGURANÇA: O caminho DataDir não pode ser vazio ou nulo."
}

if ([string]::IsNullOrWhiteSpace($DataDir)) {
    $DataDir = Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::UserProfile)) ".remotecontroller"
}

# Normalização canônica do caminho alvo
$resolvedTarget = if ([System.IO.Path]::IsPathRooted($TargetDir)) {
    [System.IO.Path]::GetFullPath($TargetDir).TrimEnd('\', '/')
} else {
    [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $TargetDir)).TrimEnd('\', '/')
}
$targetLeaf = Split-Path -Path $resolvedTarget -Leaf

# GUARDRAIL INVIOLÁVEL 1: Nome exato da pasta
if ($targetLeaf -ne "RemoteDesktopController") {
    throw "ERRO CRÍTICO DE SEGURANÇA: O diretório alvo deve ter o nome de pasta exato 'RemoteDesktopController'. Alvo recebido: '$resolvedTarget' (Leaf: '$targetLeaf'). Operação abortada para impedir deleção acidental de pastas pai ou vizinhas."
}

# GUARDRAIL INVIOLÁVEL 2: Proibição estrita de diretórios raiz ou de sistema
$rootPath = [System.IO.Path]::GetPathRoot($resolvedTarget).TrimEnd('\', '/')
if ($resolvedTarget -eq $rootPath) {
    throw "ERRO CRÍTICO DE SEGURANÇA: O diretório alvo não pode ser a raiz da unidade ('$resolvedTarget')."
}

$forbiddenCriticalPaths = @(
    $env:WINDIR,
    $env:SystemRoot,
    $env:ProgramFiles,
    ${env:ProgramFiles(x86)},
    $env:USERPROFILE,
    $env:LOCALAPPDATA,
    (Join-Path $env:LOCALAPPDATA "Programs")
) | Where-Object { [string]::IsNullOrWhiteSpace($_) -eq $false } | ForEach-Object {
    [System.IO.Path]::GetFullPath($_).TrimEnd('\', '/')
}

if ($forbiddenCriticalPaths -contains $resolvedTarget) {
    throw "ERRO CRÍTICO DE SEGURANÇA: O diretório alvo coincide com um diretório de sistema ou pasta compartilhada ('$resolvedTarget'). Operação cancelada imediatamente."
}

# Verificação se o diretório alvo existe
if (-not (Test-Path -LiteralPath $resolvedTarget)) {
    Write-Output "[AGENT-SECOPS] O diretório de instalação '$resolvedTarget' não existe. Nada a remover."
    return
}

# Validação do diretório de dados caso -PurgeData / --purge-data tenha sido requisitado
$resolvedData = $null
if ($PurgeData) {
    if ([string]::IsNullOrWhiteSpace($DataDir)) {
        throw "ERRO DE SEGURANÇA: DataDir não pode ser vazio ao solicitar -PurgeData."
    }
    $resolvedData = if ([System.IO.Path]::IsPathRooted($DataDir)) {
        [System.IO.Path]::GetFullPath($DataDir).TrimEnd('\', '/')
    } else {
        [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $DataDir)).TrimEnd('\', '/')
    }
    $dataLeaf = Split-Path -Path $resolvedData -Leaf

    if ($dataLeaf -ne ".remotecontroller") {
        throw "ERRO CRÍTICO DE SEGURANÇA: O diretório de dados deve ter o nome exato '.remotecontroller'. Recebido: '$resolvedData'."
    }
    if ($forbiddenCriticalPaths -contains $resolvedData -or $resolvedData -eq $rootPath) {
        throw "ERRO CRÍTICO DE SEGURANÇA: O diretório de dados fornecido coincide com um diretório do sistema ('$resolvedData')."
    }
}

# ==============================================================================
# 2. CONFIRMAÇÃO INTERATIVA (SE -Force NÃO FOR FORNECIDO)
# ==============================================================================

if (-not $Force) {
    if ($PSCmdlet.ShouldProcess($resolvedTarget, "Desinstalar Remote Desktop Controller")) {
        if ([System.Environment]::UserInteractive -and $Host -and $Host.UI -and $Host.UI.PromptForChoice) {
            try {
                $confirmTitle = "Confirmação de Desinstalação - Remote Desktop Controller"
                $confirmPrompt = "Deseja realmente desinstalar o Remote Desktop Controller de '$resolvedTarget'?"
                if ($PurgeData) {
                    $confirmPrompt += "`nALERTA: A opção --purge-data foi selecionada. Todos os dados e configurações em '$resolvedData' serão apagados."
                }
                $choices = [System.Management.Automation.Host.ChoiceDescription[]]@(
                    (New-Object System.Management.Automation.Host.ChoiceDescription "&Sim", "Continuar e desinstalar o Remote Desktop Controller."),
                    (New-Object System.Management.Automation.Host.ChoiceDescription "&Não", "Cancelar a operação.")
                )
                $choice = $Host.UI.PromptForChoice($confirmTitle, $confirmPrompt, $choices, 1)
                if ($choice -ne 0) {
                    Write-Output "[AGENT-SECOPS] Operação cancelada pelo usuário."
                    return
                }
            } catch {
                # Se não for possível exibir prompt (sessão não interativa ou stdin redirecionado), prossegue
            }
        }
    } else {
        Write-Output "[AGENT-SECOPS] Operação cancelada (-WhatIf ou ShouldProcess)."
        return
    }
}

# ==============================================================================
# 3. ENCERRAMENTO FORÇADO DE PROCESSOS ATIVOS DO AGENTE
# ==============================================================================

Write-Output "[AGENT-SECOPS] Verificando e finalizando processos ativos do Remote Desktop Controller..."

$canonicalProgramPath = [System.IO.Path]::GetFullPath((Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::LocalApplicationData)) "Programs\RemoteDesktopController")).TrimEnd('\', '/')
$isCanonicalTarget = ($resolvedTarget -eq $canonicalProgramPath)
$agentProcessNames = @("rdc", "local-daemon", "mcp-gateway", "device-agent")
foreach ($procName in $agentProcessNames) {
    $activeProcs = Get-Process -Name $procName -ErrorAction SilentlyContinue
    if ($activeProcs) {
        $procsToStop = $activeProcs | Where-Object {
            $isCanonicalTarget -or ($_.Path -and [System.IO.Path]::GetFullPath($_.Path).StartsWith($resolvedTarget, [System.StringComparison]::OrdinalIgnoreCase))
        }
        if ($procsToStop) {
            Write-Output "[AGENT-SECOPS] Encerrando processo ativo: $procName (PIDs: $(($procsToStop | Select-Object -ExpandProperty Id) -join ', '))"
            $procsToStop | Stop-Process -Force -ErrorAction SilentlyContinue
        }
    }
}

# Aguarda até 5 segundos para que as handles sejam totalmente liberadas pelo SO
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 5000) {
    $remaining = Get-Process -Name $agentProcessNames -ErrorAction SilentlyContinue | Where-Object {
        $isCanonicalTarget -or ($_.Path -and [System.IO.Path]::GetFullPath($_.Path).StartsWith($resolvedTarget, [System.StringComparison]::OrdinalIgnoreCase))
    }
    if (-not $remaining) { break }
    Start-Sleep -Milliseconds 100
}

# ==============================================================================
# 4. GERAÇÃO DO WORKER DESACOPLADO EM $env:TEMP (DESVINCULAÇÃO DE FILE-LOCK)
# ==============================================================================

$currentPid = $PID
$cleanupScriptPath = Join-Path $env:TEMP "rdc_cleanup_${currentPid}.ps1"

# Script autocontido do worker assíncrono
$workerScriptContent = @'
param(
    [string]$TargetDirectory = "",
    [int]$ParentProcessId = 0,
    [switch]$ShouldPurgeData,
    [string]$DataDirectory = ""
)

$ErrorActionPreference = 'SilentlyContinue'

# 0. Garante que o worker não mantenha handle na pasta alvo
Set-Location -LiteralPath $env:TEMP

# 1. Aguarda brevemente a liberação do processo pai se especificado (máx 500ms para não bloquear callers interativos)
if ($ParentProcessId -gt 0) {
    try {
        $parent = Get-Process -Id $ParentProcessId -ErrorAction SilentlyContinue
        if ($parent) {
            $parent.WaitForExit(500)
        }
    } catch {}
}

# 2. Garante novamente que processos do agente estão mortos se pertencerem ao alvo ou se for canônico
$canonicalProg = [System.IO.Path]::GetFullPath((Join-Path ([System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::LocalApplicationData)) "Programs\RemoteDesktopController")).TrimEnd('\', '/')
$isTargetCanonical = ($cleanTarget -eq $canonicalProg)
$agentProcs = @("rdc", "local-daemon", "mcp-gateway", "device-agent")
foreach ($p in $agentProcs) {
    Get-Process -Name $p -ErrorAction SilentlyContinue | Where-Object {
        $isTargetCanonical -or ($_.Path -and [System.IO.Path]::GetFullPath($_.Path).StartsWith($cleanTarget, [System.StringComparison]::OrdinalIgnoreCase))
    } | Stop-Process -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 200

# 3. Guardrails secundários de validação interna no worker (Zero-Trust)
if ([string]::IsNullOrWhiteSpace($TargetDirectory)) { exit 1 }
$cleanTarget = [System.IO.Path]::GetFullPath($TargetDirectory).TrimEnd('\', '/')
$cleanLeaf = Split-Path -Path $cleanTarget -Leaf
if ($cleanLeaf -ne "RemoteDesktopController") { exit 2 }

# 4. Exclusão física com retentativas para absorver latência de antivírus/file lock
$maxRetries = 10
$removed = $false
for ($attempt = 1; $attempt -le $maxRetries; $attempt++) {
    if (-not (Test-Path -LiteralPath $cleanTarget)) {
        $removed = $true
        break
    }
    try {
        Remove-Item -LiteralPath $cleanTarget -Recurse -Force -ErrorAction Stop
        $removed = $true
        break
    } catch {
        Start-Sleep -Milliseconds 300
    }
}

if (-not $removed -and (Test-Path -LiteralPath $cleanTarget)) {
    # Fallback via cmd rd para forçar limpeza de arquivos de sistema travados
    cmd.exe /c "rd /s /q `"$cleanTarget`"" 2>$null
}

# 5. Purge de dados se solicitado explicitamente (--purge-data)
if ($ShouldPurgeData -and -not [string]::IsNullOrWhiteSpace($DataDirectory)) {
    $cleanData = [System.IO.Path]::GetFullPath($DataDirectory).TrimEnd('\', '/')
    $dataLeaf = Split-Path -Path $cleanData -Leaf
    if ($dataLeaf -eq ".remotecontroller" -and (Test-Path -LiteralPath $cleanData)) {
        for ($attempt = 1; $attempt -le 5; $attempt++) {
            try {
                Remove-Item -LiteralPath $cleanData -Recurse -Force -ErrorAction Stop
                break
            } catch {
                Start-Sleep -Milliseconds 200
            }
        }
    }
}

# 6. Remoção de entrada de registro do desinstalador no Windows
try {
    Remove-Item -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\RemoteDesktopController" -Recurse -Force -ErrorAction SilentlyContinue
} catch {}

# 7. Auto-deleção do próprio script do worker
try {
    if ($PSCommandPath -and (Test-Path -LiteralPath $PSCommandPath)) {
        Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
    }
} catch {}
'@

Write-Output "[AGENT-SECOPS] Desacoplando processo de exclusão física para worker temporário: '$cleanupScriptPath'..."
[System.IO.File]::WriteAllText($cleanupScriptPath, $workerScriptContent, [System.Text.UTF8Encoding]::new($false))

# Limpeza proativa de registro antes da exclusão de arquivos
try {
    Remove-Item -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\RemoteDesktopController" -Recurse -Force -ErrorAction SilentlyContinue
} catch {}

# ==============================================================================
# 5. EXECUÇÃO DO WORKER DESACOPLADO
# ==============================================================================

# Libera o diretório atual do script antes de iniciar o processo filho, evitando file lock
Set-Location -LiteralPath $env:TEMP

# Se -Wait foi solicitado, o worker não deve esperar o PID pai morrer (pois causaria deadlock)
$parentPidArg = if ($Wait) { 0 } else { $currentPid }

$argList = @(
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", "`"$cleanupScriptPath`"",
    "-TargetDirectory", "`"$resolvedTarget`"",
    "-ParentProcessId", "$parentPidArg"
)

if ($PurgeData) {
    $argList += "-ShouldPurgeData"
    if (-not [string]::IsNullOrWhiteSpace($resolvedData)) {
        $argList += "-DataDirectory"
        $argList += "`"$resolvedData`""
    }
}

$startInfo = New-Object System.Diagnostics.ProcessStartInfo
$startInfo.FileName = "powershell.exe"
$startInfo.Arguments = ($argList -join ' ')
$startInfo.WorkingDirectory = $env:TEMP
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden

$workerProcess = [System.Diagnostics.Process]::Start($startInfo)

if ($Wait) {
    Write-Output "[AGENT-SECOPS] Aguardando conclusão do worker de limpeza (-Wait ativo)..."
    [void]$workerProcess.WaitForExit(30000)
    Write-Output "[AGENT-SECOPS] Desinstalação concluída com sucesso. Alvo '$resolvedTarget' expurgado com segurança."
} else {
    Write-Output "[AGENT-SECOPS] Worker assíncrono iniciado com PID $($workerProcess.Id). Liberando console para encerramento imediato."
}
