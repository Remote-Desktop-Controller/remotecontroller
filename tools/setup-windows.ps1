#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$PackageDirectory = $PSScriptRoot,
    [string]$InstallRoot,
    [string]$ProfileDirectory,
    [uri]$ServerUrl,
    [string]$Workspace,
    [string]$Name = 'Meu computador',
    [switch]$ChooseWorkspace,
    [switch]$FullComputer,
    [switch]$AllowWrite,
    [switch]$AllowProcess,
    [switch]$AllowProcessNetwork,
    [switch]$AllowProcessOutsideWorkspace,
    [switch]$EnableAutostart,
    [switch]$DisableAutostart,
    [switch]$StartAgent,
    [switch]$AllowHttpLoopback,
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
function Assert-NoReparsePath([string]$Path) {
    $taskPath = [IO.Path]::GetFullPath($Path)
    while ($taskPath) {
        if (Test-Path -LiteralPath $taskPath) {
            $taskItem = Get-Item -LiteralPath $taskPath -Force
            if ($taskItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {throw "Links/junctions não são permitidos: $taskPath"}
        }
        $taskParent = [IO.Path]::GetDirectoryName($taskPath)
        if ($taskParent -eq $taskPath) {break}
        $taskPath = $taskParent
    }
}
function Test-Within([string]$Path, [string]$Parent) {
    $taskPath = [IO.Path]::GetFullPath($Path).TrimEnd('\','/')
    $taskParent = [IO.Path]::GetFullPath($Parent).TrimEnd('\','/')
    # Rust canonical Windows paths use extended prefixes; compare the same namespace.
    if ($taskPath.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {$taskPath = '\\' + $taskPath.Substring(8)}
    elseif ($taskPath.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {$taskPath = $taskPath.Substring(4)}
    if ($taskParent.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {$taskParent = '\\' + $taskParent.Substring(8)}
    elseif ($taskParent.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {$taskParent = $taskParent.Substring(4)}
    return $taskPath.Equals($taskParent, [StringComparison]::OrdinalIgnoreCase) -or $taskPath.StartsWith($taskParent + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}
function Read-VerifiedManifest([string]$Root) {
    Assert-NoReparsePath $Root
    $taskManifestPath = Join-Path $Root 'manifest.json'
    Assert-NoReparsePath $taskManifestPath
    $taskInfo = Get-Item -LiteralPath $taskManifestPath
    if ($taskInfo.PSIsContainer -or $taskInfo.Length -gt 65536) {throw 'Manifesto inválido ou excede 64 KiB'}
    $taskManifest = Get-Content -LiteralPath $taskManifestPath -Raw | ConvertFrom-Json
    if ($taskManifest.format -ne 1 -or $taskManifest.product -ne 'remote-desktop-controller' -or $taskManifest.platform -ne 'windows' -or $taskManifest.version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?$' -or $taskManifest.architecture -notin @('x64','arm64')) {throw 'Manifesto não é um pacote Windows reconhecido'}
    $taskProperties = @($taskManifest.files.PSObject.Properties)
    if ($taskProperties.Count -lt 10 -or $taskProperties.Count -gt 64) {throw 'Catálogo de arquivos inválido'}
    $taskNames = @{}
    foreach ($taskFile in $taskProperties) {
        $taskRelative = $taskFile.Name
        if ($taskRelative -notmatch '^[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*(?:/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*)*$' -or $taskFile.Value -isnot [string] -or $taskFile.Value -notmatch '^[a-fA-F0-9]{64}$') {throw "Caminho/checksum inválido: $taskRelative"}
        if ($taskNames.ContainsKey($taskRelative.ToLowerInvariant())) {throw 'Caminhos duplicados no manifesto'}
        $taskNames[$taskRelative.ToLowerInvariant()] = $true
        $taskPath = Join-Path $Root $taskRelative
        if (-not (Test-Within $taskPath $Root)) {throw 'Caminho fora do pacote'}
        Assert-NoReparsePath $taskPath
        $taskFileInfo = Get-Item -LiteralPath $taskPath
        if ($taskFileInfo.PSIsContainer -or (Get-FileHash -LiteralPath $taskPath -Algorithm SHA256).Hash -ne $taskFile.Value) {throw "Checksum inválido: $taskRelative"}
    }
    foreach ($taskRequired in @('local-daemon.exe','mcp-gateway.exe','device-agent.exe','remote-service.exe','setup-windows.ps1','INSTALL.md','OWNER.md','LICENSE','LICENSES/libsql.md','LICENSES/libsql-patch.md','LICENSES/portable-pty.md','LICENSES/portable-pty-patch.md')) {
        if (-not $taskNames.ContainsKey($taskRequired.ToLowerInvariant())) {throw "Arquivo obrigatório ausente: $taskRequired"}
    }
    return $taskManifest
}
function Invoke-Agent([string[]]$Arguments) {
    & $script:taskAgent @Arguments
    if ($LASTEXITCODE -ne 0) {throw "device-agent falhou (exit $LASTEXITCODE); perfil e versões foram preservados"}
}
if ($EnableAutostart -and $DisableAutostart) {throw 'Escolha EnableAutostart ou DisableAutostart'}
if ($ServerUrl) {
    $taskAddress = $null
    $taskLoopback = $ServerUrl.DnsSafeHost -eq 'localhost' -or ([Net.IPAddress]::TryParse($ServerUrl.DnsSafeHost.Trim('[',']'), [ref]$taskAddress) -and [Net.IPAddress]::IsLoopback($taskAddress))
    if (-not $ServerUrl.IsAbsoluteUri -or $ServerUrl.UserInfo -or $ServerUrl.Query -or $ServerUrl.Fragment -or $ServerUrl.AbsolutePath -ne '/' -or ($ServerUrl.Scheme -ne 'https' -and -not ($AllowHttpLoopback -and $taskLoopback -and $ServerUrl.Scheme -eq 'http'))) {throw 'ServerUrl exige origem HTTPS; HTTP apenas loopback explicitamente autorizado'}
}
$taskPackage = (Resolve-Path -LiteralPath $PackageDirectory).Path
$taskManifest = Read-VerifiedManifest $taskPackage
if ($VerifyOnly) {
    [pscustomobject]@{verified=$true;version=$taskManifest.version;architecture=$taskManifest.architecture;package=$taskPackage;file_count=@($taskManifest.files.PSObject.Properties).Count;effects='none'} | ConvertTo-Json
    return
}
if ($env:OS -ne 'Windows_NT' -or -not $env:LOCALAPPDATA) {throw 'Instalação exige Windows e LOCALAPPDATA; VerifyOnly apenas verifica arquivos'}
$taskMachineArchitecture = if ($env:PROCESSOR_ARCHITEW6432) {$env:PROCESSOR_ARCHITEW6432} else {$env:PROCESSOR_ARCHITECTURE}
$taskExpectedArchitecture = if ($taskMachineArchitecture -eq 'ARM64') {'arm64'} elseif ($taskMachineArchitecture -eq 'AMD64') {'x64'} else {throw 'Arquitetura Windows não suportada'}
if ($taskManifest.architecture -ne $taskExpectedArchitecture) {throw "Pacote $($taskManifest.architecture) não corresponde ao Windows $taskExpectedArchitecture"}
if (-not $InstallRoot) {$InstallRoot = Join-Path $env:LOCALAPPDATA 'Programs/RemoteDesktopController'}
if (-not $ProfileDirectory) {$ProfileDirectory = Join-Path $env:LOCALAPPDATA 'RemoteDesktopController/agent'}
$taskInstallRoot = [IO.Path]::GetFullPath($InstallRoot)
$taskProfile = [IO.Path]::GetFullPath($ProfileDirectory)
if (-not (Test-Within $taskInstallRoot $env:LOCALAPPDATA) -or $taskInstallRoot.Equals([IO.Path]::GetFullPath($env:LOCALAPPDATA), [StringComparison]::OrdinalIgnoreCase)) {throw 'InstallRoot deve ser uma subpasta de LOCALAPPDATA'}
if ((Test-Within $taskProfile $taskInstallRoot) -or (Test-Within $taskInstallRoot $taskProfile)) {throw 'Perfil e instalação devem usar pastas separadas'}
Assert-NoReparsePath $taskInstallRoot
Assert-NoReparsePath $taskProfile
if ($ChooseWorkspace) {
    if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {throw 'Folder picker exige PowerShell -STA; ou informe -Workspace'}
    Add-Type -AssemblyName System.Windows.Forms
    $taskPicker = New-Object Windows.Forms.FolderBrowserDialog
    try {
        $taskPicker.Description = 'Escolha a pasta de trabalho autorizada no computador'
        $taskPicker.ShowNewFolderButton = $false
        if ($taskPicker.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) {throw 'Seleção de pasta cancelada; nenhuma instalação realizada'}
        $Workspace = $taskPicker.SelectedPath
    } finally {$taskPicker.Dispose()}
}
if ($Workspace) {
    $Workspace = (Resolve-Path -LiteralPath $Workspace).Path
    if (-not (Get-Item -LiteralPath $Workspace).PSIsContainer) {throw 'Workspace deve ser uma pasta existente'}
    Assert-NoReparsePath $Workspace
    if (-not $FullComputer -and ((Test-Within $taskProfile $Workspace) -or (Test-Within $taskInstallRoot $Workspace))) {throw 'Perfil e executáveis devem permanecer fora do workspace'}
}
$taskCredentials = Join-Path $taskProfile 'agent.json'
Assert-NoReparsePath $taskCredentials
if (Test-Path -LiteralPath $taskCredentials) {
    $taskProfileInfo = Get-Item -LiteralPath $taskCredentials
    if ($taskProfileInfo.PSIsContainer -or $taskProfileInfo.Length -gt 16384) {throw 'Perfil existente inválido; não foi alterado'}
    # Read only the workspace to protect the installation from existing tool access.
    $taskExistingProfile = Get-Content -LiteralPath $taskCredentials -Raw | ConvertFrom-Json
    $taskExistingWorkspace = [IO.Path]::GetFullPath([string]$taskExistingProfile.workspace)
    if (-not $FullComputer -and ((Test-Within $taskProfile $taskExistingWorkspace) -or (Test-Within $taskInstallRoot $taskExistingWorkspace))) {throw 'Perfil/instalação ficam dentro do workspace do perfil existente'}
    $taskExistingProfile = $null
}
if ($ServerUrl -and -not (Test-Path -LiteralPath $taskCredentials) -and -not $Workspace) {throw 'Pareamento exige -Workspace ou -ChooseWorkspace'}
if (($EnableAutostart -or $StartAgent) -and -not (Test-Path -LiteralPath $taskCredentials) -and -not $ServerUrl) {throw 'Pareie primeiro: informe ServerUrl e Workspace'}
$taskVersionRoot = Join-Path $taskInstallRoot $taskManifest.version
Assert-NoReparsePath $taskVersionRoot
if (Test-Path -LiteralPath $taskVersionRoot) {
    $taskPrevious = Read-VerifiedManifest $taskVersionRoot
    if ($taskPrevious.version -ne $taskManifest.version -or $taskPrevious.architecture -ne $taskManifest.architecture -or @($taskPrevious.files.PSObject.Properties).Count -ne @($taskManifest.files.PSObject.Properties).Count) {throw 'Versão existente diverge; nenhuma substituição realizada'}
    foreach ($taskFile in $taskManifest.files.PSObject.Properties) {
        if ($taskPrevious.files.PSObject.Properties[$taskFile.Name].Value -ne $taskFile.Value) {throw 'Mesma versão contém arquivos diferentes; use nova versão'}
    }
} else {
    # Refuse an upgrade while this profile is active; do not stop it automatically.
    if (Test-Path -LiteralPath (Join-Path $taskProfile 'agent.lock')) {
        $taskLock = [IO.File]::Open((Join-Path $taskProfile 'agent.lock'), [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $taskLock.Dispose()
    }
    New-Item -ItemType Directory -Path $taskInstallRoot -Force | Out-Null
    $taskStage = Join-Path $taskInstallRoot ('.stage-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $taskStage | Out-Null
    foreach ($taskFile in $taskManifest.files.PSObject.Properties) {
        $taskDestination = Join-Path $taskStage $taskFile.Name
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($taskDestination)) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $taskPackage $taskFile.Name) -Destination $taskDestination
        if ((Get-FileHash -LiteralPath $taskDestination -Algorithm SHA256).Hash -ne $taskFile.Value) {throw "Checksum mudou durante instalação: $($taskFile.Name)"}
    }
    Copy-Item -LiteralPath (Join-Path $taskPackage 'manifest.json') -Destination (Join-Path $taskStage 'manifest.json')
    $null = Read-VerifiedManifest $taskStage
    if (-not (Test-Within $taskStage $taskInstallRoot) -or -not (Test-Within $taskVersionRoot $taskInstallRoot)) {throw 'Staging fora da instalação'}
    Move-Item -LiteralPath $taskStage -Destination $taskVersionRoot
}
$script:taskAgent = Join-Path $taskVersionRoot 'device-agent.exe'
if (Test-Path -LiteralPath $taskCredentials) {
    Write-Output 'Perfil existente preservado. Servidor, workspace e permissões não foram substituídos.'
} elseif ($ServerUrl) {
    $taskPairArgs = @('--profile', $taskProfile, 'pair', '--server', $ServerUrl.AbsoluteUri.TrimEnd('/'), '--workspace', $Workspace, '--name', $Name)
    foreach ($taskFlag in @(@($AllowWrite,'--allow-write'),@($AllowProcess,'--allow-process'),@($AllowProcessNetwork,'--allow-process-network'),@($AllowProcessOutsideWorkspace,'--allow-process-outside-workspace'),@($AllowHttpLoopback,'--allow-http-loopback'),@($FullComputer,'--full-computer'))) {
        if ($taskFlag[0]) {$taskPairArgs += $taskFlag[1]}
    }
    Invoke-Agent $taskPairArgs
}
if ($EnableAutostart) {Invoke-Agent @('--profile', $taskProfile, 'autostart', 'enable')}
if ($DisableAutostart) {Invoke-Agent @('--profile', $taskProfile, 'autostart', 'disable')}
if ($StartAgent) {
    $taskProcess = Start-Process -FilePath $script:taskAgent -ArgumentList @('--profile', ('"' + $taskProfile + '"'), 'run') -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 2
    if ($taskProcess.HasExited) {throw 'Agente encerrou ao iniciar; execute run no terminal para diagnóstico'}
    Write-Output "Processo do agente iniciado: PID $($taskProcess.Id). Confirme presença na conta."
}
Write-Output "Arquivos instalados e verificados em $taskVersionRoot"
Write-Output "Perfil preservado em $taskProfile"
Write-Output "Diagnóstico: & '$script:taskAgent' --profile '$taskProfile' status"
Write-Output "Parar: & '$script:taskAgent' --profile '$taskProfile' stop"
Write-Output 'remote-service foi incluído para implantação separada; não foi iniciado no computador.'
