#requires -Version 7.0
param([string]$BinaryDirectory=(Join-Path $PSScriptRoot '../target/release'))
$ErrorActionPreference='Stop'
if (-not $IsWindows) { throw 'Este gate exercita instalação e registro Windows' }
$taskFixture=Join-Path $env:LOCALAPPDATA ('Temp/rdc-product-validation-'+[guid]::NewGuid().ToString('N'))
$taskOutput=Join-Path $taskFixture 'packages'
$taskInstall=Join-Path $taskFixture 'installed'
$taskProfile=Join-Path $taskFixture 'profile with spaces'
$taskKey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$taskBefore=@((Get-Item -LiteralPath $taskKey).Property)
New-Item -ItemType Directory -Path $taskProfile -Force | Out-Null
Set-Content -LiteralPath (Join-Path $taskProfile 'preserve.txt') -Value 'preserve fixture' -Encoding utf8NoBOM
& (Join-Path $PSScriptRoot 'package-product.ps1') -BinaryDirectory $BinaryDirectory -OutputDirectory $taskOutput | Out-Null
$taskPackage=Join-Path $taskOutput 'remote-desktop-controller-windows-x64-0.3.0'
$taskSetup=Join-Path $taskPackage 'setup-windows.ps1'
$taskWindowsPowerShell=Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
& $taskWindowsPowerShell -NoProfile -File $taskSetup -PackageDirectory $taskPackage -VerifyOnly
if ($LASTEXITCODE -ne 0) { throw 'VerifyOnly falhou no PowerShell 5.1' }
& $taskSetup -PackageDirectory $taskPackage -InstallRoot $taskInstall -ProfileDirectory $taskProfile | Out-Null
& $taskSetup -PackageDirectory $taskPackage -InstallRoot $taskInstall -ProfileDirectory $taskProfile | Out-Null
if ((Get-Content -LiteralPath (Join-Path $taskProfile 'preserve.txt') -Raw).Trim() -ne 'preserve fixture') { throw 'Estado local foi alterado' }
$taskAgent=Join-Path $taskInstall '0.3.0/device-agent.exe'
try {
    & $taskAgent --profile $taskProfile autostart enable
    if ($LASTEXITCODE -ne 0) { throw 'Ativação autostart falhou' }
    $taskAdded=@((Get-Item -LiteralPath $taskKey).Property | Where-Object { $_ -notin $taskBefore -and $_ -like 'remotecontroller-*' })
    if ($taskAdded.Count -ne 1) { throw 'Entrada autostart ausente/ambígua' }
    $taskCommand=(Get-ItemProperty -LiteralPath $taskKey).($taskAdded[0])
    if ($taskCommand -notmatch '-WindowStyle Hidden -EncodedCommand ([A-Za-z0-9+/=]+)$') { throw 'Comando autostart inválido' }
    $taskDecoded=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($Matches[1]))
    if (-not $taskDecoded.Contains('device-agent.exe') -or -not $taskDecoded.Contains('profile with spaces') -or -not $taskDecoded.Contains('-WindowStyle Hidden')) { throw 'Autostart perde perfil/esconde mal a janela' }
} finally {
    & $taskAgent --profile $taskProfile autostart disable
    if ($LASTEXITCODE -ne 0) { throw 'Falha removendo autostart da fixture' }
}
foreach ($taskName in $taskAdded) { if ($taskName -in (Get-Item -LiteralPath $taskKey).Property) { throw 'Autostart de fixture não removido' } }
$taskTampered=Join-Path $taskPackage 'LICENSE'
Add-Content -LiteralPath $taskTampered -Value 'tamper-test'
$taskRejected=$false
try { & $taskSetup -PackageDirectory $taskPackage -VerifyOnly | Out-Null } catch { $taskRejected=$true }
if (-not $taskRejected) { throw 'Checksum alterado não foi recusado' }
Write-Output 'PASS: pacote, PowerShell 5.1 VerifyOnly, instalação/reinstalação, preservação, autostart com espaços/remoção, checksum adulterado'
Write-Output "Fixture preservada para auditoria: $taskFixture"
