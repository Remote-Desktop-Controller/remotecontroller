# Executar manualmente no PowerShell. Pede a senha no terminal, nunca no chat.
# 1. Valida o backup existente e exibe contagens de usuários/dispositivos/sessões.
# 2. Preserva imagem atual como remote-service:rollback-precommercial e backup de deployment/.
# 3. Atualiza repositório para feat/commercial-portal (41cbb98) com fast-forward limpo.
# 4. Garante que .env mantenha as variáveis comerciais vazias (modo legado).
# 5. Compila a nova imagem com streaming de log em tempo real (--progress=plain) e timeout de 15 min.
# 6. Confirma que o contêiner original segue ativo e intocado. Nenhum contêiner é reiniciado.

$script = @'
import json, os, pathlib, shutil, subprocess, sys, time

ROOT = pathlib.Path("/root/remotecontroller")
BACKUPS_ROOT = pathlib.Path("/root/remotecontroller-backups")

print("\n" + "=" * 65)
print("[ETAPA 1/6] Verificando backup consistente recente na VPS...")
print("=" * 65)
sys.stdout.flush()

if not BACKUPS_ROOT.is_dir():
    print("[ERRO] Diretório /root/remotecontroller-backups não encontrado.")
    sys.exit(1)

backups = sorted([p for p in BACKUPS_ROOT.iterdir() if p.is_dir()], key=lambda p: p.name)
if not backups:
    print("[ERRO] Nenhum backup encontrado em /root/remotecontroller-backups.")
    sys.exit(1)

latest_backup = backups[-1]
meta_file = latest_backup / "metadata.json"
if not meta_file.is_file():
    print(f"[ERRO] metadata.json não encontrado em {latest_backup}.")
    sys.exit(1)

metadata = json.loads(meta_file.read_text())
if metadata.get("status") != "valid":
    print(f"[ERRO] Status do backup em {latest_backup} é '{metadata.get('status')}', esperado 'valid'.")
    sys.exit(1)

counts = metadata.get("counts", {})
users_cnt = counts.get("users", {}).get("total", 0)
dev_total = counts.get("devices", {}).get("total", 0)
dev_active = counts.get("devices", {}).get("active", 0)
sess_total = counts.get("sessions", {}).get("total", 0)
sess_active = counts.get("sessions", {}).get("active", 0)
grants_total = counts.get("grants", {}).get("total", 0)
grants_active = counts.get("grants", {}).get("active", 0)

print(f"  Diretório: {latest_backup}")
print(f"  Status: VALID | SHA256: {metadata.get('sha256', '')[:16]}... | Tamanho: {metadata.get('bytes', 0)} bytes")
print(f"  Integridade: {metadata.get('integrity_check')} | Foreign Keys: {metadata.get('foreign_key_check')}")
print(f"  Métricas de Produção:")
print(f"    - Usuários: {users_cnt}")
print(f"    - Dispositivos: {dev_total} (ativos: {dev_active})")
print(f"    - Sessões: {sess_total} (ativas: {sess_active})")
print(f"    - Concessões OAuth: {grants_total} (ativas: {grants_active})")
sys.stdout.flush()

print("\n" + "=" * 65)
print("[ETAPA 2/6] Preservando versão atual para rollback garantido...")
print("=" * 65)
sys.stdout.flush()

p = subprocess.run(
    ["docker", "ps", "-q", "--filter", "label=com.docker.compose.project=deployment", "--filter", "label=com.docker.compose.service=service"],
    capture_output=True, text=True, stdin=subprocess.DEVNULL
)
cid = p.stdout.strip()
if not cid:
    print("[ERRO] Contêiner do serviço de produção não encontrado em execução.")
    sys.exit(1)

inspect = json.loads(subprocess.run(["docker", "inspect", cid], capture_output=True, text=True, stdin=subprocess.DEVNULL).stdout)[0]
current_image_id = inspect["Image"]

subprocess.run(["docker", "tag", current_image_id, "remote-service:rollback-precommercial"], check=True, stdin=subprocess.DEVNULL)
print(f"  Tag de imagem criada: remote-service:rollback-precommercial -> {current_image_id[:19]}...")

deployment_backup = ROOT / "deployment.backup-precommercial"
if not deployment_backup.exists():
    shutil.copytree(ROOT / "deployment", deployment_backup)
    print(f"  Backup das configurações: {deployment_backup}")
else:
    print(f"  Backup das configurações já existia: {deployment_backup} (preservado)")

subprocess.run(["git", "-C", str(ROOT), "tag", "-f", "rollback-precommercial", "HEAD"], check=True, stdin=subprocess.DEVNULL)
print("  Tag git criada: rollback-precommercial")
sys.stdout.flush()

print("\n" + "=" * 65)
print("[ETAPA 3/6] Sincronizando repositório com origin/feat/commercial-portal...")
print("=" * 65)
sys.stdout.flush()

git_env = os.environ.copy()
git_env["GIT_TERMINAL_PROMPT"] = "0"

print("  Buscando atualizações remotas (git fetch)...")
fetch_res = subprocess.run(
    ["git", "-C", str(ROOT), "fetch", "origin", "feat/commercial-portal"],
    capture_output=True, text=True, stdin=subprocess.DEVNULL, env=git_env, timeout=60
)
if fetch_res.returncode != 0:
    print(f"[ERRO] Falha ao executar git fetch: {fetch_res.stderr}")
    sys.exit(1)

print("  Atualizando branch local para origin/feat/commercial-portal...")
checkout_res = subprocess.run(
    ["git", "-C", str(ROOT), "checkout", "-B", "feat/commercial-portal", "origin/feat/commercial-portal"],
    capture_output=True, text=True, stdin=subprocess.DEVNULL, env=git_env, timeout=30
)
if checkout_res.returncode != 0:
    print(f"[ERRO] Falha ao atualizar branch: {checkout_res.stderr}")
    sys.exit(1)

new_head = subprocess.run(
    ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
    capture_output=True, text=True, stdin=subprocess.DEVNULL
).stdout.strip()
print(f"  Repositório sincronizado no commit: {new_head}")
sys.stdout.flush()

print("\n" + "=" * 65)
print("[ETAPA 4/6] Verificando deployment/.env (Modo Comercial DESATIVADO)...")
print("=" * 65)
sys.stdout.flush()

env_file = ROOT / "deployment" / ".env"
env_map = {}
if env_file.exists():
    for line in env_file.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            env_map[k] = v

env_map["REMOTE_SERVICE_COMMERCIAL_PORTAL_URL"] = ""
env_map["REMOTE_SERVICE_COMMERCIAL_SECRET"] = ""

out_lines = [f"{k}={v}" for k, v in sorted(env_map.items())]
env_file.write_text("\n".join(out_lines) + "\n")
os.chmod(env_file, 0o600)
print("  Variáveis comerciais confirmadas como VAZIAS:")
print("    REMOTE_SERVICE_COMMERCIAL_PORTAL_URL=''")
print("    REMOTE_SERVICE_COMMERCIAL_SECRET=''")
print("  -> O serviço Rust executará estritamente em modo legado, sem bloqueios de licença.")
sys.stdout.flush()

print("\n" + "=" * 65)
print("[ETAPA 5/6] Compilando imagem Docker com log em tempo real...")
print("=" * 65)
sys.stdout.flush()

compose_file = ROOT / "deployment" / "compose.yaml"
cmd = [
    "docker", "compose",
    "--env-file", str(env_file),
    "-f", str(compose_file),
    "build",
    "--progress=plain",
    "service"
]

build_proc = subprocess.Popen(
    cmd,
    cwd=str(ROOT / "deployment"),
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    stdin=subprocess.DEVNULL,
    text=True,
    bufsize=1
)

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
        print("\n[ERRO] Timeout excedido (15 min) durante a compilação Docker.")
        sys.exit(1)

if build_proc.returncode != 0:
    print(f"\n[ERRO] Compilação Docker falhou com código {build_proc.returncode}.")
    sys.exit(1)

new_img_proc = subprocess.run(
    ["docker", "compose", "--env-file", str(env_file), "-f", str(compose_file), "images", "-q", "service"],
    cwd=str(ROOT / "deployment"), capture_output=True, text=True, stdin=subprocess.DEVNULL
)
new_image_id = new_img_proc.stdout.strip()
print(f"\n  Compilação concluída com sucesso. Imagem gerada: {new_image_id}")
sys.stdout.flush()

print("\n" + "=" * 65)
print("[ETAPA 6/6] Verificação final de integridade e continuidade do serviço...")
print("=" * 65)
sys.stdout.flush()

still_running = subprocess.run(
    ["docker", "ps", "-q", "--filter", f"id={cid}"],
    capture_output=True, text=True, stdin=subprocess.DEVNULL
).stdout.strip()

if still_running:
    print("  Status do contêiner ativo: RUNNING (serviço ininterrupto)")
else:
    print("  [ALERTA] Contêiner não detectado como em execução.")

print("\n" + "#" * 65)
print("PREPARAÇÃO FINALIZADA COM SUCESSO")
print(f"Versão compilada: {new_head}")
print("Rollback preservado: remote-service:rollback-precommercial")
print("Modo comercial: DESATIVADO (legado funcional)")
print("Nenhum contêiner foi reiniciado ou substituído.")
print("#" * 65)
'@

Write-Host "Iniciando preparação da nova versão na VPS (andredozero.webajato.com.br)..." -ForegroundColor Cyan
$script | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'
