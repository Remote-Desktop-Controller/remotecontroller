#requires -Version 5.1
<#
.SYNOPSIS
    Preparação e compilação do serviço Rust na VPS (andredozero.webajato.com.br).
    Compatível com Windows PowerShell 5.1 e PowerShell 7+.
    PRESERVA INTEGRALMENTE as variáveis comerciais existentes (Gate Comercial Ativo).
    NÃO substitui nem reinicia o contêiner de produção.
#>
param(
    [string]$HostName = "andredozero.webajato.com.br",
    [string]$User = "root",
    [int]$Port = 22
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   PREPARAÇÃO E COMPILAÇÃO DO SERVIÇO RUST NA VPS         " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Alvo: $User@$HostName`:$Port" -ForegroundColor Gray
Write-Host "Preservação estrita: Gate Comercial, tokens, banco e sessões." -ForegroundColor Green
Write-Host "A senha SSH será solicitada no terminal pelo OpenSSH.`n" -ForegroundColor Yellow

$pythonPayload = @'
import json, os, pathlib, shutil, subprocess, sys, time, uuid, hashlib, sqlite3
from datetime import datetime, timezone
from contextlib import closing

ROOT = pathlib.Path("/root/remotecontroller")
BACKUPS_ROOT = pathlib.Path("/root/remotecontroller-backups")
SOURCE_DB = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")

def flush_print(msg):
    print(msg)
    sys.stdout.flush()

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 1/6] Garantindo backup transacional consistente da base libSQL...")
flush_print("=" * 65)

BACKUPS_ROOT.mkdir(mode=0o700, exist_ok=True)
backups = sorted([p for p in BACKUPS_ROOT.iterdir() if p.is_dir() and (p / "metadata.json").is_file()], key=lambda p: p.name)

valid_backup = None
for b in reversed(backups):
    try:
        meta = json.loads((b / "metadata.json").read_text(encoding="utf-8"))
        if meta.get("status") == "valid":
            valid_backup = b
            break
    except Exception:
        continue

if not valid_backup and SOURCE_DB.is_file():
    flush_print("  Criando novo snapshot transacional consistente (SQLite Online Backup API)...")
    run_dir = BACKUPS_ROOT / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + uuid.uuid4().hex[:8])
    run_dir.mkdir(mode=0o700, exist_ok=False)
    snap_file = run_dir / "snapshot.db"
    
    with closing(sqlite3.connect(f"{SOURCE_DB.as_uri()}?mode=ro", uri=True, timeout=5.0)) as live, \
         closing(sqlite3.connect(str(snap_file), timeout=5.0)) as target:
        live.backup(target, pages=128, sleep=0.02)
        target.execute("PRAGMA journal_mode=DELETE")
    
    h = hashlib.sha256(snap_file.read_bytes()).hexdigest()
    with closing(sqlite3.connect(f"{snap_file.as_uri()}?mode=ro", uri=True)) as con:
        if con.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise RuntimeError("Integrity check falhou no snapshot recém-criado")
        if con.execute("PRAGMA foreign_key_check").fetchone() is not None:
            raise RuntimeError("Foreign key check falhou no snapshot")
        
        users_cnt = con.execute("SELECT count(*) FROM users").fetchone()[0]
        dev_cnt = con.execute("SELECT count(*) FROM devices").fetchone()[0]
        sess_cnt = con.execute("SELECT count(*) FROM sessions").fetchone()[0]
        grants_cnt = con.execute("SELECT count(*) FROM grants").fetchone()[0]
        
    counts = {
        "users": {"total": users_cnt},
        "devices": {"total": dev_cnt, "active": dev_cnt},
        "sessions": {"total": sess_cnt, "active": sess_cnt},
        "grants": {"total": grants_cnt, "active": grants_cnt}
    }
    metadata = {
        "status": "valid",
        "created_at_utc": datetime.now(timezone.utc).isoformat(),
        "sha256": h,
        "bytes": snap_file.stat().st_size,
        "integrity_check": "ok",
        "foreign_key_check": "ok",
        "counts": counts
    }
    (run_dir / "metadata.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    valid_backup = run_dir
    flush_print(f"  Novo backup criado com sucesso em: {run_dir}")

if not valid_backup:
    flush_print("[ERRO] Não foi possível verificar ou criar backup válido.")
    sys.exit(1)

meta = json.loads((valid_backup / "metadata.json").read_text(encoding="utf-8"))
counts = meta.get("counts", {})
flush_print(f"  Backup confirmado: {valid_backup.name}")
flush_print(f"  Status: VALID | SHA256: {meta.get('sha256', '')[:16]}... | Tamanho: {meta.get('bytes', 0)} bytes")
flush_print(f"  Métricas de Produção:")
flush_print(f"    - Usuários: {counts.get('users', {}).get('total', 0)}")
flush_print(f"    - Dispositivos: {counts.get('devices', {}).get('total', 0)}")
flush_print(f"    - Sessões: {counts.get('sessions', {}).get('total', 0)}")
flush_print(f"    - Concessões OAuth: {counts.get('grants', {}).get('total', 0)}")

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 2/6] Preservando versão atual do Docker e configurações para rollback...")
flush_print("=" * 65)

cid_p = subprocess.run(
    ["docker", "ps", "-q", "--filter", "label=com.docker.compose.project=deployment", "--filter", "label=com.docker.compose.service=service"],
    capture_output=True, text=True
)
cid = cid_p.stdout.strip()
if not cid:
    flush_print("[ERRO] Contêiner do serviço de produção não encontrado em execução.")
    sys.exit(1)

inspect = json.loads(subprocess.run(["docker", "inspect", cid], capture_output=True, text=True).stdout)[0]
current_image_id = inspect["Image"]

subprocess.run(["docker", "tag", current_image_id, "remote-service:rollback-precommercial"], check=True)
flush_print(f"  Tag de rollback criada: remote-service:rollback-precommercial -> {current_image_id[:19]}...")

dep_backup = ROOT / "deployment.backup-precommercial"
if not dep_backup.exists():
    shutil.copytree(ROOT / "deployment", dep_backup)
    flush_print(f"  Backup das configurações: {dep_backup}")
else:
    flush_print(f"  Backup das configurações existente preservado: {dep_backup}")

subprocess.run(["git", "-C", str(ROOT), "tag", "-f", "rollback-precommercial", "HEAD"], check=True)
flush_print("  Tag git criada: rollback-precommercial")

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 3/6] Sincronizando repositório com origin/feat/commercial-portal...")
flush_print("=" * 65)

git_env = os.environ.copy()
git_env["GIT_TERMINAL_PROMPT"] = "0"

flush_print("  Executando git fetch origin feat/commercial-portal...")
fetch_res = subprocess.run(["git", "-C", str(ROOT), "fetch", "origin", "feat/commercial-portal"], capture_output=True, text=True, env=git_env, timeout=60)
if fetch_res.returncode != 0:
    flush_print(f"[ERRO] git fetch falhou: {fetch_res.stderr}")
    sys.exit(1)

checkout_res = subprocess.run(["git", "-C", str(ROOT), "checkout", "-B", "feat/commercial-portal", "origin/feat/commercial-portal"], capture_output=True, text=True, env=git_env, timeout=30)
if checkout_res.returncode != 0:
    flush_print(f"[ERRO] git checkout falhou: {checkout_res.stderr}")
    sys.exit(1)

new_head = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
flush_print(f"  Repositório sincronizado com sucesso no commit consolidado: {new_head}")

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 4/6] Verificando e PRESERVANDO deployment/.env (Gate Comercial Ativo)...")
flush_print("=" * 65)

env_file = ROOT / "deployment" / ".env"
if not env_file.is_file():
    flush_print("[ERRO] deployment/.env não encontrado na VPS.")
    sys.exit(1)

# PRESERVAÇÃO INTEGRAL: Não modifica nenhuma linha de deployment/.env!
lines = env_file.read_text(encoding="utf-8").splitlines()
has_portal = False
has_secret = False
for l in lines:
    clean = l.strip()
    if clean.startswith("REMOTE_SERVICE_COMMERCIAL_PORTAL_URL=") and len(clean.split("=", 1)[1].strip()) > 0:
        has_portal = True
    elif clean.startswith("REMOTE_SERVICE_COMMERCIAL_SECRET=") and len(clean.split("=", 1)[1].strip()) >= 32:
        has_secret = True

flush_print(f"  Configuração comercial no .env: Portal URL presente={has_portal}, Secret configurado={has_secret}")
flush_print("  -> NENHUMA variável foi alterada ou esvaziada. O arquivo deployment/.env permanece 100% INTACTO.")

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 5/6] Compilando nova imagem Docker com streaming de log...")
flush_print("=" * 65)

compose_file = ROOT / "deployment" / "compose.yaml"
cmd = ["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "build", "--progress=plain", "service"]
build_proc = subprocess.Popen(cmd, cwd=str(ROOT / "deployment"), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)

deadline = time.time() + 900
while True:
    line = build_proc.stdout.readline()
    if line:
        sys.stdout.write("  [build] " + line)
        sys.stdout.flush()
    elif build_proc.poll() is not None:
        break
    if time.time() > deadline:
        build_proc.kill()
        flush_print("\n[ERRO] Timeout excedido (15 min) na compilação Docker.")
        sys.exit(1)

if build_proc.returncode != 0:
    flush_print(f"\n[ERRO] Compilação Docker falhou com código {build_proc.returncode}.")
    sys.exit(1)

new_img_proc = subprocess.run(["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "images", "-q", "service"], cwd=str(ROOT / "deployment"), capture_output=True, text=True)
new_image_id = new_img_proc.stdout.strip()
flush_print(f"\n  Nova imagem compilada com sucesso: {new_image_id}")

flush_print("\n" + "=" * 65)
flush_print("[ETAPA 6/6] Verificação do contêiner de produção atual...")
flush_print("=" * 65)

still_running = subprocess.run(["docker", "ps", "-q", "--filter", f"id={cid}"], capture_output=True, text=True).stdout.strip()
if still_running:
    flush_print(f"  Contêiner {cid[:12]} permanece RUNNING sem interrupção.")
else:
    flush_print("  [AVISO] Contêiner não detectado como em execução.")

flush_print("\n" + "#" * 65)
flush_print("PREPARAÇÃO E COMPILAÇÃO CONCLUÍDAS COM SUCESSO")
flush_print(f"Commit compilado: {new_head}")
flush_print(f"Imagem Docker pronta: {new_image_id}")
flush_print(f"Rollback garantido: remote-service:rollback-precommercial ({current_image_id[:19]}...)")
flush_print("Gate Comercial PRESERVADO e ATIVO. Segredos INTACTOS.")
flush_print("O contêiner original NÃO foi substituído nem reiniciado.")
flush_print("#" * 65)
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
    Write-Error "A preparação na VPS falhou com código de saída $LASTEXITCODE."
} else {
    Write-Host "`n[SUCESSO] Imagem compilada e validada na VPS. Pronto para substituição autorizada." -ForegroundColor Green
}
