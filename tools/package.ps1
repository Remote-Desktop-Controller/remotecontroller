param(
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../target/release'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../dist'),
    [string]$Version = '0.2.0'
)
$ErrorActionPreference = 'Stop'
$platform = if ($IsLinux) { 'linux' } elseif ($IsMacOS) { 'macos' } else { 'windows' }
$extension = if ($platform -eq 'windows') { '.exe' } else { '' }
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$bundleName = "local-runtime-$platform-$architecture-$Version"
$bundle = Join-Path $OutputDirectory $bundleName
if (Test-Path -LiteralPath $bundle) { throw "Bundle already exists: $bundle" }
New-Item -ItemType Directory -Path $bundle -Force | Out-Null
$files = [ordered]@{}
foreach ($name in @("local-daemon$extension", "mcp-gateway$extension")) {
    $source = Join-Path $BinaryDirectory $name
    $destination = Join-Path $bundle $name
    Copy-Item -LiteralPath $source -Destination $destination
    $files[$name] = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
}
$manifest = @{ version = $Version; files = $files } | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText((Join-Path $bundle 'manifest.json'), $manifest, [System.Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../docs/install.md') -Destination (Join-Path $bundle 'INSTALL.md')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../LICENSE') -Destination (Join-Path $bundle 'LICENSE')
$archive = Join-Path $OutputDirectory "$bundleName.zip"
Compress-Archive -Path (Join-Path $bundle '*') -DestinationPath $archive
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $bundleName.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
Write-Output $archive
