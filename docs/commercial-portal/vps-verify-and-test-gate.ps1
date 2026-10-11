# Script de Diagnostico, Vinculacao e Teste Controlado do Gate Comercial
# Compativel com Windows PowerShell 5.1 e PowerShell 7+

$ErrorActionPreference = "Continue"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " DIAGNOSTICO E TESTE CONTROLADO DO GATE COMERCIAL           " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# 1. Validar Vercel (Producao)
Write-Host "`n[1/4] Verificando integracao na Vercel (https://remotecontroller-portal.vercel.app)..." -ForegroundColor Yellow
$cfg = Invoke-RestMethod -Uri "https://remotecontroller-portal.vercel.app/api/config" -Method Get
Write-Host "  remoteOrigin: $($cfg.remoteOrigin)" -ForegroundColor Gray
Write-Host "  serviceAvailable: $($cfg.serviceAvailable)" -ForegroundColor Gray
Write-Host "  salesEnabled: $($cfg.salesEnabled)" -ForegroundColor Gray

$mcpProbe = curl.exe -s -w "%{http_code}" -X POST "https://remotecontroller-portal.vercel.app/api/mcp/exchange"
if ($mcpProbe -match "401") {
    Write-Host "  MCP_SHARED_SECRET na Vercel: ATIVO E VALIDADO (retorna 401 Unauthorized para requisicao sem token, confirmando presenca do segredo)." -ForegroundColor Green
} elseif ($mcpProbe -match "503") {
    Write-Host "  MCP_SHARED_SECRET na Vercel: PENDENTE (retorna 503 CONFIGURATION)." -ForegroundColor Red
}

# 2. Diagnostico de Contas na VPS via SSH
Write-Host "`n[2/4] Verificando contas na VPS (andredozero.webajato.com.br)..." -ForegroundColor Yellow

$sshCheckPy = @'
import json, pathlib, sqlite3, sys

DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")
if not DB_PATH.exists():
    print(json.dumps({"error": "Database not found"}))
    sys.exit(1)

con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
cur = con.cursor()
users = cur.execute("SELECT id, email FROM users").fetchall()

try:
    comm = cur.execute("SELECT user_id, subject FROM commercial_accounts").fetchall()
except Exception:
    comm = []
con.close()

comm_map = {c[0]: c[1] for c in comm}
result = {
    "users": [
        {"id": u[0], "email": u[1], "subject": comm_map.get(u[0])}
        for u in users
    ]
}
print(json.dumps(result))
'@

$checkLines = $sshCheckPy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'
$sshExit = $LASTEXITCODE

if ($sshExit -ne 0) {
    Write-Error "Falha ao conectar na VPS via SSH."
    exit 1
}

$checkJson = ($checkLines | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
$checkData = $checkJson | ConvertFrom-Json

$unlinkedCount = 0
foreach ($u in $checkData.users) {
    if ($u.subject) {
        Write-Host "  Conta: $($u.email) -> VINCULADA ao Supabase UUID: $($u.subject)" -ForegroundColor Green
    } else {
        $unlinkedCount++
        Write-Host "  Conta: $($u.email) (ID: $($u.id)) -> PENDENTE DE VINCULO" -ForegroundColor Yellow
    }
}

# 3. Vincular conta se estiver pendente
if ($unlinkedCount -gt 0) {
    Write-Host "`n[3/4] Vinculacao de Conta Pendente:" -ForegroundColor Yellow
    $unlinkedUser = $checkData.users | Where-Object { -not $_.subject } | Select-Object -First 1
    $linkChoice = Read-Host "Deseja vincular a conta '$($unlinkedUser.email)' agora? (s/N)"
    if ($linkChoice -eq 's' -or $linkChoice -eq 'S') {
        $subUuid = Read-Host "Cole o UUID correspondente do Supabase Auth (ex: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)"
        if ($subUuid -match '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
            $sshDoLinkPy = @'
import pathlib, sqlite3, sys
user_id = sys.argv[1]
subject = sys.argv[2]
DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")
con = sqlite3.connect(str(DB_PATH))
cur = con.cursor()
cur.execute("INSERT INTO commercial_accounts(user_id, subject) VALUES(?, ?) ON CONFLICT(user_id) DO UPDATE SET subject=excluded.subject", (user_id, subject))
con.commit()
con.close()
print("LINK_OK")
'@
            $linkRes = $sshDoLinkPy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br "python3 -B - '$($unlinkedUser.id)' '$subUuid'"
            if ($linkRes -match "LINK_OK") {
                Write-Host "  Conta vinculada com sucesso a '$subUuid'!" -ForegroundColor Green
            } else {
                Write-Host "  Erro na vinculacao: $linkRes" -ForegroundColor Red
            }
        } else {
            Write-Host "  UUID invalido fornecido. Vinculo mantido pendente." -ForegroundColor Yellow
        }
    } else {
        Write-Host "  Vinculacao mantida como pendente. Nenhuma alteracao realizada." -ForegroundColor Yellow
    }
} else {
    Write-Host "`n[3/4] Nenhuma conta pendente de vinculacao encontrada." -ForegroundColor Green
}

# 4. Opcao de Ativacao Controlada do Gate Comercial na VPS com Rollback Instantaneo
Write-Host "`n[4/4] Ativacao Controlada do Gate Comercial na VPS:" -ForegroundColor Yellow
Write-Host "O gate comercial direciona /login para o portal e exige licenca ativa para comandos." -ForegroundColor Gray
Write-Host "Se o teste falhar ou se voce desejar retornar ao modo legado, o rollback e instantaneo." -ForegroundColor Gray

$activateChoice = Read-Host "Deseja ATIVAR o gate comercial na VPS agora para realizacao do teste? (s/N)"
if ($activateChoice -eq 's' -or $activateChoice -eq 'S') {
    # Ler o segredo compartilhado que foi configurado na Vercel (ou solicitar)
    $secretInput = Read-Host "Cole o segredo compartilhado (ou pressione Enter para gerar/usar novo segredo sincronizado)"
    if (-not $secretInput) {
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        $b = New-Object byte[] 32
        $rng.GetBytes($b)
        $secretInput = [System.BitConverter]::ToString($b).Replace("-", "").ToLowerInvariant()
        Write-Host "  Atualizando segredo na Vercel com novo valor gerado..." -ForegroundColor Yellow
        $portalDir = "C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\apps\portal"
        $vercelCmd = (Get-Command "vercel.cmd" -ErrorAction SilentlyContinue).Source
        if (-not $vercelCmd) { $vercelCmd = "vercel.cmd" }
        Set-Location -LiteralPath $portalDir
        $secretInput | & $vercelCmd env add MCP_SHARED_SECRET production --force --sensitive
        & $vercelCmd deploy --prod --yes --scope andrefonns-projects --project remotecontroller-portal --non-interactive --no-color
    }

    $sshActivatePy = @'
import pathlib, subprocess, sys

ROOT = pathlib.Path("/root/remotecontroller")
ENV_PATH = ROOT / "deployment" / ".env"
portal_url = "https://remotecontroller-portal.vercel.app"
secret = sys.argv[1]

lines = ENV_PATH.read_text(encoding="utf-8").splitlines()
new_lines = []
for line in lines:
    if line.startswith("REMOTE_SERVICE_COMMERCIAL_PORTAL_URL="):
        new_lines.append(f"REMOTE_SERVICE_COMMERCIAL_PORTAL_URL={portal_url}")
    elif line.startswith("REMOTE_SERVICE_COMMERCIAL_SECRET="):
        new_lines.append(f"REMOTE_SERVICE_COMMERCIAL_SECRET={secret}")
    else:
        new_lines.append(line)

ENV_PATH.write_text("\n".join(new_lines) + "\n", encoding="utf-8")

res = subprocess.run(
    ["docker", "compose", "--env-file", str(ENV_PATH), "-f", str(ROOT / "deployment" / "compose.yaml"), "up", "-d", "service"],
    capture_output=True, text=True
)
if res.returncode != 0:
    print("RESTART_FAILED: " + res.stderr)
    sys.exit(1)
print("GATE_ACTIVE")
'@

    $actRes = $sshActivatePy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br "python3 -B - '$secretInput'"
    if ($actRes -match "GATE_ACTIVE") {
        Write-Host "  Gate comercial ATIVADO na VPS com sucesso." -ForegroundColor Green
        
        # Testar efeito imediato
        $loginTest = curl.exe -s -i "https://andredozero.webajato.com.br/login"
        if ($loginTest -match "303" -and $loginTest -match "remotecontroller-portal\.vercel\.app/connect") {
            Write-Host "  [OK] /login redireciona 303 para o portal comercial!" -ForegroundColor Green
        } else {
            Write-Host "  [AVISO] /login nao redirecionou como esperado." -ForegroundColor Yellow
        }

        # Oferecer Rollback Imediato
        Write-Host "`n------------------------------------------------------------" -ForegroundColor Cyan
        $rbChoice = Read-Host "Deseja reverter a VPS para o MODO LEGADO agora? (S/n)"
        if ($rbChoice -ne 'n' -and $rbChoice -ne 'N') {
            $sshRollbackPy = @'
import pathlib, subprocess
ROOT = pathlib.Path("/root/remotecontroller")
ENV_PATH = ROOT / "deployment" / ".env"
lines = ENV_PATH.read_text(encoding="utf-8").splitlines()
new_lines = []
for line in lines:
    if line.startswith("REMOTE_SERVICE_COMMERCIAL_PORTAL_URL="):
        new_lines.append("REMOTE_SERVICE_COMMERCIAL_PORTAL_URL=")
    elif line.startswith("REMOTE_SERVICE_COMMERCIAL_SECRET="):
        new_lines.append("REMOTE_SERVICE_COMMERCIAL_SECRET=")
    else:
        new_lines.append(line)
ENV_PATH.write_text("\n".join(new_lines) + "\n", encoding="utf-8")
subprocess.run(["docker", "compose", "--env-file", str(ENV_PATH), "-f", str(ROOT / "deployment" / "compose.yaml"), "up", "-d", "service"], check=True)
print("ROLLBACK_OK")
'@
            $rbRes = $sshRollbackPy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'
            if ($rbRes -match "ROLLBACK_OK") {
                Write-Host "  [ROLLBACK CONCLUIDO] VPS retornada com sucesso ao modo legado." -ForegroundColor Green
            }
        } else {
            Write-Host "  Gate comercial mantido ativo na VPS." -ForegroundColor Yellow
        }
    } else {
        Write-Host "  Falha ao ativar gate na VPS: $actRes" -ForegroundColor Red
    }
} else {
    Write-Host "  Ativacao suspensa. VPS permanece em modo legado." -ForegroundColor Yellow
}

Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host " DIAGNOSTICO CONCLUIDO COM SUCESSO.                         " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
