param(
    [Parameter(Mandatory)][uri]$ServiceUrl,
    [Parameter(Mandatory)][string]$DeveloperName,
    [Parameter(Mandatory)][uri]$WebsiteUrl,
    [Parameter(Mandatory)][uri]$SupportUrl,
    [Parameter(Mandatory)][uri]$PrivacyPolicyUrl,
    [Parameter(Mandatory)][uri]$TermsOfServiceUrl,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../dist/plugin')
)
$ErrorActionPreference = 'Stop'
foreach ($taskUrl in @($ServiceUrl,$WebsiteUrl,$SupportUrl,$PrivacyPolicyUrl,$TermsOfServiceUrl)) {
    if ($taskUrl.Scheme -ne 'https' -or $taskUrl.UserInfo -or $taskUrl.Fragment) { throw 'URLs públicas devem usar HTTPS e não conter credenciais/fragmentos' }
}
if ($ServiceUrl.AbsolutePath -ne '/' -or $ServiceUrl.Query) { throw 'ServiceUrl precisa ser apenas a origem HTTPS' }
$taskRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$taskStage = Join-Path $OutputDirectory ('stage-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $taskStage -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $taskRoot 'plugin/skills') -Destination $taskStage -Recurse
Copy-Item -LiteralPath (Join-Path $taskRoot 'plugin/assets') -Destination $taskStage -Recurse
$taskManifest = Get-Content -LiteralPath (Join-Path $taskRoot 'plugin/plugin.json') -Raw | ConvertFrom-Json
$taskInterface = $taskManifest.extensions.'com.openai'.interface
foreach ($taskField in @{developerName=$DeveloperName;websiteURL=$WebsiteUrl.AbsoluteUri;supportURL=$SupportUrl.AbsoluteUri;privacyPolicyURL=$PrivacyPolicyUrl.AbsoluteUri;termsOfServiceURL=$TermsOfServiceUrl.AbsoluteUri}.GetEnumerator()) {
    $taskInterface | Add-Member -NotePropertyName $taskField.Key -NotePropertyValue $taskField.Value -Force
}
$taskManifest | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $taskStage 'plugin.json') -Encoding utf8NoBOM
$taskMcp = Get-Content -LiteralPath (Join-Path $taskRoot 'plugin/mcp.json') -Raw | ConvertFrom-Json
$taskMcp.mcpServers.'remote-desktop-controller'.url = $ServiceUrl.AbsoluteUri.TrimEnd('/') + '/mcp'
$taskMcp | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $taskStage 'mcp.json') -Encoding utf8NoBOM
Copy-Item -LiteralPath (Join-Path $taskRoot 'plugin/review-cases.json') -Destination $taskStage
$taskArchive = Join-Path $OutputDirectory ('remote-desktop-controller-' + $taskManifest.version + '-' + [guid]::NewGuid().ToString('N') + '.zip')
Compress-Archive -LiteralPath (Get-ChildItem -LiteralPath $taskStage | Select-Object -ExpandProperty FullName) -DestinationPath $taskArchive
$taskHash = Get-FileHash -LiteralPath $taskArchive -Algorithm SHA256
Write-Output ('Pacote preparado: ' + $taskHash.Path)
Write-Output ('SHA256: ' + $taskHash.Hash)
Write-Output 'O pacote requer revisão/publicação na plataforma; não é confirmação de aprovação.'
