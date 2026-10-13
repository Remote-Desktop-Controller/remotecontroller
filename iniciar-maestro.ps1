Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "          INICIANDO RDC MAESTRO (DESKTOP)               " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan
Write-Host ""

$env:PATH = "$HOME\.cargo\bin;$env:PATH"
Set-Location -Path "$PSScriptRoot\apps\rdc-maestro"

Write-Host "[1/2] Iniciando Vite e compilador Tauri v2..." -ForegroundColor Yellow
Write-Host "[2/2] A janela nativa do desktop abrira em instantes..." -ForegroundColor Green
Write-Host ""

npm run tauri dev
