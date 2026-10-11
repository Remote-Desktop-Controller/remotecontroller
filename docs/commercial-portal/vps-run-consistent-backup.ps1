# Executar manualmente no PowerShell. Pede a senha no terminal, nunca no chat.
# Cria snapshot transacionalmente consistente em tempo quente com WAL ativo,
# valida integridade física e foreign keys, testa restauração em arquivo isolado
# e retorna auditoria agregada de usuários/dispositivos/sessões sem expor segredos.

$scriptPath = Join-Path $PSScriptRoot "vps-consistent-backup.py"
if (-not (Test-Path -LiteralPath $scriptPath)) {
    Write-Error "Script Python não encontrado em: $scriptPath"
    exit 1
}

$pyScript = Get-Content -LiteralPath $scriptPath -Raw -Encoding utf8

$sshCommand = "python3 -B - --source /var/lib/docker/volumes/deployment_service-data/_data/service.db --destination-root /root/remotecontroller-backups --project-root /root/remotecontroller --service-uid 10001 --timeout 60"

Write-Host "Iniciando backup consistente na VPS (andredozero.webajato.com.br)..." -ForegroundColor Cyan
$pyScript | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br $sshCommand
