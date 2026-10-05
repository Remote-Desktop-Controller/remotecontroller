param([string]$Path = (Join-Path $PSScriptRoot '../docs/product/status.json'))
$ErrorActionPreference = 'Stop'
$taskStatus = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
$taskWeight = ($taskStatus.tasks | Measure-Object -Property weight -Sum).Sum
if ($taskWeight -ne 100) { throw "Pesos inválidos: $taskWeight" }
$taskEarned = [int](($taskStatus.tasks | Where-Object status -eq 'verified' | Measure-Object -Property weight -Sum).Sum)
Write-Output "Prontidão técnica por marcos verificados: $taskEarned/100"
Write-Output "Produto público validado: $($taskStatus.public_product_validated)"
$taskStatus.tasks | Select-Object id,title,weight,status | Format-Table -AutoSize
Write-Output $taskStatus.score_definition
