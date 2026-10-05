#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../target/release'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../dist'),
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?$')][string]$Version = '0.3.0',
    [ValidateSet('windows','linux','macos')][string]$Platform = $(if ($IsLinux) {'linux'} elseif ($IsMacOS) {'macos'} else {'windows'}),
    [ValidateSet('x64','arm64')][string]$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant(),
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$taskRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$taskExtension = if ($Platform -eq 'windows') {'.exe'} else {''}
$taskSources = [ordered]@{}
foreach ($taskName in @('local-daemon','mcp-gateway','device-agent','remote-service')) {
    $taskSources["$taskName$taskExtension"] = Join-Path $BinaryDirectory "$taskName$taskExtension"
    $taskNativePlatform = if ($IsLinux) {'linux'} elseif ($IsMacOS) {'macos'} else {'windows'}
    if ($Platform -eq $taskNativePlatform) {
        $taskVersionOutput = & $taskSources["$taskName$taskExtension"] --version
        if ($LASTEXITCODE -ne 0 -or "$taskVersionOutput".Trim() -ne "$taskName $Version") {throw "Versão divergente: $taskName; esperado $Version, recebido $taskVersionOutput"}
    }
}
$taskSources['setup-windows.ps1'] = Join-Path $taskRoot 'tools/setup-windows.ps1'
$taskSources['INSTALL.md'] = Join-Path $taskRoot 'docs/product/install-product.md'
$taskSources['OWNER.md'] = Join-Path $taskRoot 'docs/product/manual-owner.md'
$taskSources['LICENSE'] = Join-Path $taskRoot 'LICENSE'
$taskSources['LICENSES/libsql.md'] = Join-Path $taskRoot 'vendor/libsql/LICENSE.md'
$taskSources['LICENSES/libsql-patch.md'] = Join-Path $taskRoot 'vendor/libsql/PATCH.md'
$taskSources['LICENSES/portable-pty.md'] = Join-Path $taskRoot 'vendor/portable-pty/LICENSE.md'
$taskSources['LICENSES/portable-pty-patch.md'] = Join-Path $taskRoot 'vendor/portable-pty/PATCH.md'
$taskHashes = [ordered]@{}
foreach ($taskEntry in $taskSources.GetEnumerator()) {
    $taskItem = Get-Item -LiteralPath $taskEntry.Value
    if ($taskItem.PSIsContainer -or ($taskItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw "Fonte precisa ser arquivo regular: $($taskEntry.Value)"}
    $taskHashes[$taskEntry.Key] = (Get-FileHash -LiteralPath $taskItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
$taskBundleName = "remote-desktop-controller-$Platform-$Architecture-$Version"
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
$taskBundle = Join-Path $taskOutput $taskBundleName
$taskArchive = "$taskBundle.zip"
if ($VerifyOnly) {
    [pscustomobject]@{verified=$true;version=$Version;platform=$Platform;architecture=$Architecture;files=$taskHashes;planned_archive=$taskArchive} | ConvertTo-Json -Depth 5
    return
}
if ((Test-Path -LiteralPath $taskBundle) -or (Test-Path -LiteralPath $taskArchive) -or (Test-Path -LiteralPath "$taskArchive.sha256")) {throw "Pacote já existe; use outra versão/pasta: $taskBundle"}
New-Item -ItemType Directory -Path $taskBundle -Force | Out-Null
foreach ($taskEntry in $taskSources.GetEnumerator()) {
    $taskDestination = Join-Path $taskBundle $taskEntry.Key
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($taskDestination)) -Force | Out-Null
    Copy-Item -LiteralPath $taskEntry.Value -Destination $taskDestination
    if ((Get-FileHash -LiteralPath $taskDestination -Algorithm SHA256).Hash.ToLowerInvariant() -ne $taskHashes[$taskEntry.Key]) {throw "Checksum mudou durante a cópia: $($taskEntry.Key)"}
}
$taskManifest = [ordered]@{format=1;product='remote-desktop-controller';version=$Version;platform=$Platform;architecture=$Architecture;files=$taskHashes}
[IO.File]::WriteAllText((Join-Path $taskBundle 'manifest.json'), ($taskManifest | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
Compress-Archive -LiteralPath (Get-ChildItem -LiteralPath $taskBundle | Select-Object -ExpandProperty FullName) -DestinationPath $taskArchive
$taskArchiveHash = (Get-FileHash -LiteralPath $taskArchive -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$taskArchive.sha256", "$taskArchiveHash  $taskBundleName.zip`n", [Text.Encoding]::ASCII)
Write-Output $taskArchive
Write-Output "SHA256: $taskArchiveHash"
Write-Output 'Checksums verificam integridade; assinatura e publicação exigem evidência separada.'
