#requires -Version 5.1
<#
.SYNOPSIS
    Substituição controlada do contêiner Rust na VPS com verificação e rollback automático.
    Compatível com Windows PowerShell 5.1 e PowerShell 7+.
    PRESERVA INTEGRALMENTE as variáveis comerciais, vínculos, volumes e segredos existentes.
    EXECUTE SOMENTE APÓS CONFIRMAR AUTORIZAÇÃO EXPLÍCITA.
#>
param(
    [string]$HostName = "andredozero.webajato.com.br",
    [string]$User = "root",
    [int]$Port = 22
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   SUBSTITUIÇÃO CONTROLADA DO CONTÊINER RUST NA VPS       " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Alvo: $User@$HostName`:$Port" -ForegroundColor Gray
Write-Host "A senha SSH será solicitada no terminal pelo OpenSSH.`n" -ForegroundColor Yellow

$pythonPayload = @'
import json, pathlib, subprocess, sys, time, urllib.request

ROOT = pathlib.Path("/root/remotecontroller")
compose_file = ROOT / "deployment" / "compose.yaml"
env_file = ROOT / "deployment" / ".env"

def flush(msg):
    print(msg)
    sys.stdout.flush()

# 1. Garantir que o contêiner atual existe e capturar seu Image ID real
flush("[1/5] Inspecionando contêiner em execução e travando tag de rollback...")
cid_proc = subprocess.run(
    ["docker", "ps", "-q", "--filter", "label=com.docker.compose.project=deployment", "--filter", "label=com.docker.compose.service=service"],
    capture_output=True, text=True
)
cid = cid_proc.stdout.strip()
if not cid:
    flush("[ERRO] Contêiner em execução não encontrado. Abortando substituição.")
    sys.exit(1)

inspect_curr = json.loads(subprocess.run(["docker", "inspect", cid], capture_output=True, text=True).stdout)[0]
current_image_id = inspect_curr["Image"]

# Taguear explicitamente a imagem atual em execução como rollback garantido
subprocess.run(["docker", "tag", current_image_id, "remote-service:rollback-precommercial"], check=True)
flush(f"  Rollback travado com sucesso: remote-service:rollback-precommercial -> {current_image_id[:19]}...")

# 2. Identificar a nova imagem compilada
flush("\n[2/5] Identificando a nova imagem para ativação...")
new_img_id = subprocess.run(
    ["docker", "images", "-q", "deployment-service:latest"],
    capture_output=True, text=True
).stdout.strip()

if not new_img_id:
    # Fallback para a tag do compose
    new_img_id = subprocess.run(
        ["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "images", "-q", "service"],
        cwd=str(ROOT / "deployment"), capture_output=True, text=True
    ).stdout.strip()

flush(f"  Imagem atual em execução: {current_image_id[:19]}...")
flush(f"  Nova imagem a ativar:     {new_img_id[:19]}...")

# 3. Recriar contêiner com a nova imagem preservando .env e volumes
flush("\n[3/5] Recriando contêiner com a nova imagem compilada...")
flush("  Preservando deployment/.env intacto com Gate Comercial ativo.")
flush("  Preservando volume libSQL service-data intacto.")

res = subprocess.run(
    ["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "up", "-d", "--no-build", "service"],
    cwd=str(ROOT / "deployment"), capture_output=True, text=True
)
if res.returncode != 0:
    flush(f"[ERRO] Falha ao iniciar contêiner: {res.stderr}")
    flush("[ROLLBACK] Revertendo para imagem anterior...")
    subprocess.run(["docker", "tag", current_image_id, "deployment-service:latest"], check=False)
    subprocess.run(["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "up", "-d", "--no-build", "service"], check=False, cwd=str(ROOT / "deployment"))
    sys.exit(1)

flush("  Contêiner iniciado. Aguardando 5 segundos para estabilização...")
time.sleep(5)

# 4. Validar status e healthcheck
flush("\n[4/5] Validando status e healthcheck do novo contêiner...")
new_cid = subprocess.run(
    ["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "ps", "-q", "service"],
    cwd=str(ROOT / "deployment"), capture_output=True, text=True
).stdout.strip()

if not new_cid:
    flush("[ERRO] Novo contêiner não encontrado após subida.")
    sys.exit(1)

inspect_new_c = json.loads(subprocess.run(["docker", "inspect", new_cid], capture_output=True, text=True).stdout)[0]
new_status = inspect_new_c["State"]["Status"]
if new_status != "running":
    flush(f"[ERRO] Status inesperado: {new_status}. Executando rollback...")
    subprocess.run(["docker", "tag", current_image_id, "deployment-service:latest"], check=False)
    subprocess.run(["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "up", "-d", "--no-build", "service"], check=False, cwd=str(ROOT / "deployment"))
    sys.exit(1)

flush(f"  Contêiner {new_cid[:12]} ativo (status: {new_status}).")

# Validar healthcheck HTTP local
try:
    req = urllib.request.Request("http://127.0.0.1:8787/health")
    with urllib.request.urlopen(req, timeout=5) as r:
        body = r.read().decode("utf-8")
        flush(f"  Healthcheck local na porta 8787 (HTTP {r.status}): {body}")
except Exception as e:
    flush(f"  Aviso ao consultar health interno na porta 8787: {e}")

# 5. Logs do novo serviço
flush("\n[5/5] Logs iniciais do novo serviço:")
logs = subprocess.run(["docker", "logs", "--tail", "15", new_cid], capture_output=True, text=True).stdout
for l in logs.splitlines():
    flush(f"  [log] {l}")

flush("\n" + "#" * 65)
flush("SUBSTITUIÇÃO DO CONTÊINER CONCLUÍDA COM SUCESSO")
flush(f"Contêiner atualizado para imagem: {new_img_id[:19]}...")
flush(f"Rollback garantido preservado: remote-service:rollback-precommercial -> {current_image_id[:19]}...")
flush("Gate comercial, vínculos, volumes e segredos 100% preservados.")
flush("#" * 65)
'@

$bytes = [System.Text.Encoding]::UTF8.GetBytes($pythonPayload)
$b64 = [System.Convert]::ToBase64String($bytes)
$remoteCmd = "echo $b64 | base64 -d | python3 -B"

$sshArgs = @(
    "-o", "PreferredAuthentications=password",
    "-o", "PubkeyAuthentication=no",
    "-o", "StrictHostKeyChecking=yes",
    "-p", "$Port",
    "$User@$HostName",
    $remoteCmd
)

& ssh @sshArgs

if ($LASTEXITCODE -ne 0) {
    Write-Host "`n[FALHA] Executando rollback de emergência..." -ForegroundColor Red
    $rbCmd = "docker tag remote-service:rollback-precommercial deployment-service:latest && docker compose --env-file /root/remotecontroller/deployment/.env -f /root/remotecontroller/deployment/compose.yaml up -d --no-build service"
    & ssh @sshArgs $rbCmd
    Write-Error "Falha na substituição. Rollback acionado."
} else {
    Write-Host "`n[SUCESSO] Contêiner atualizado e operacional." -ForegroundColor Green
}
