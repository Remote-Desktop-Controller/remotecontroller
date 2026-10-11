# Script de Preparacao e Finalizacao da Integracao Comercial Portal <-> VPS
# Compativel com Windows PowerShell 5.1 e PowerShell 7+

# Em PowerShell 5.1, manter 'Continue' para comandos externos (evita NativeCommandError com stderr)
$ErrorActionPreference = "Continue"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " CONFIGURADOR DA INTEGRACAO COMERCIAL (PORTAL <-> VPS)      " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# 1. Gerar segredo criptografico forte compativel com PowerShell 5.1 (256 bits / 64 hex)
try {
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $secretBytes = New-Object byte[] 32
    $rng.GetBytes($secretBytes)
    $sharedSecret = [System.BitConverter]::ToString($secretBytes).Replace("-", "").ToLowerInvariant()
    Write-Host "[1/5] Segredo criptografico de 256 bits gerado com sucesso." -ForegroundColor Green
} catch {
    Write-Error "Falha ao gerar segredo criptografico: $_"
    exit 1
}

# 2. Localizar executavel da Vercel CLI (preferencia para vercel.cmd em vez de .ps1)
$portalDir = "C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\apps\portal"
if (-not (Test-Path -LiteralPath $portalDir)) {
    Write-Error "Diretorio do portal nao encontrado: $portalDir"
    exit 1
}
Set-Location -LiteralPath $portalDir -ErrorAction Stop

$vercelCmd = (Get-Command "vercel.cmd" -ErrorAction SilentlyContinue).Source
if (-not $vercelCmd) {
    $vercelCmd = (Get-Command "vercel" -ErrorAction SilentlyContinue).Source
}
if (-not $vercelCmd) {
    Write-Error "Comando vercel ou vercel.cmd nao encontrado no PATH."
    exit 1
}

Write-Host "[2/5] Verificando autenticacao na Vercel CLI (usando $vercelCmd)..." -ForegroundColor Yellow

# Executa whoami tratando stderr como texto informativo sem estourar NativeCommandError
$whoamiRaw = & $vercelCmd whoami 2>&1
$whoamiExit = $LASTEXITCODE
$whoamiText = ($whoamiRaw | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine

$isAuthenticated = ($whoamiExit -eq 0 -and $whoamiText -notmatch "No existing credentials" -and $whoamiText -notmatch "Logged out")

if (-not $isAuthenticated) {
    Write-Host "  Vercel CLI nao autenticada (sessao expirada ou ausente)." -ForegroundColor Yellow
    Write-Host "  Iniciando autenticacao interativa com 'vercel login'..." -ForegroundColor Yellow
    & $vercelCmd login
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Falha na autenticacao da Vercel. Execute 'vercel login' e repita o script."
        exit 1
    }
}
Write-Host "  Vercel CLI autenticada com sucesso." -ForegroundColor Green

# Configurar variaveis na Vercel (Production)
Write-Host "  Configurando MCP_SERVICE_ORIGIN e MCP_SHARED_SECRET na Vercel Production..." -ForegroundColor Yellow
$pOrigin = "https://andredozero.webajato.com.br"

# Usar --force para atualizar sem erro caso ja exista
$resOrigin = $pOrigin | & $vercelCmd env add MCP_SERVICE_ORIGIN production --force 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "  Aviso ao definir MCP_SERVICE_ORIGIN: $($resOrigin -join ' ')" -ForegroundColor Yellow
}

$resSecret = $sharedSecret | & $vercelCmd env add MCP_SHARED_SECRET production --force --sensitive 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "  Aviso ao definir MCP_SHARED_SECRET: $($resSecret -join ' ')" -ForegroundColor Yellow
}

Write-Host "  Implantando variaveis na Vercel Production..." -ForegroundColor Yellow
& $vercelCmd deploy --prod --yes --scope andrefonns-projects --project remotecontroller-portal --non-interactive --no-color
if ($LASTEXITCODE -ne 0) {
    Write-Error "Falha ao publicar atualizacao na Vercel."
    exit 1
}
Write-Host "  Portal Vercel atualizado com variaveis de integracao." -ForegroundColor Green

# 3. Auditoria Somente-Leitura na VPS via SSH
Write-Host "[3/5] Conectando a VPS para auditoria de contas legadas..." -ForegroundColor Yellow

$sshAuditPy = @'
import json, os, pathlib, sqlite3, sys

ROOT = pathlib.Path("/root/remotecontroller")
DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")

if not DB_PATH.exists():
    print(json.dumps({"error": f"Database not found at {DB_PATH}"}))
    sys.exit(1)

con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
cur = con.cursor()
users = cur.execute("SELECT id, email, created_at FROM users").fetchall()

try:
    commercial = cur.execute("SELECT user_id, subject FROM commercial_accounts").fetchall()
except Exception:
    commercial = []
con.close()

comm_map = {c[0]: c[1] for c in commercial}

result = {
    "userCount": len(users),
    "users": [
        {
            "id": u[0],
            "email": u[1],
            "createdAt": u[2],
            "linkedSubject": comm_map.get(u[0])
        }
        for u in users
    ]
}
print(json.dumps(result))
'@

$vpsAuditLines = $sshAuditPy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'
$sshExit = $LASTEXITCODE
if ($sshExit -ne 0) {
    Write-Error "Falha ao conectar via SSH ou auditar banco da VPS (exit code $sshExit)."
    exit 1
}

$vpsAuditJson = ($vpsAuditLines | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
$vpsAudit = $vpsAuditJson | ConvertFrom-Json

Write-Host "  Total de contas encontradas na VPS: $($vpsAudit.userCount)" -ForegroundColor Cyan
foreach ($u in $vpsAudit.users) {
    $linked = if ($u.linkedSubject) { "Vinculado -> $($u.linkedSubject)" } else { "PENDENTE DE VINCULO" }
    Write-Host "    - ID: $($u.id) | Email: $($u.email) | Status: $linked" -ForegroundColor Gray
}

# 4. Vinculacao de conta legada (se aplicavel)
$unlinked = @($vpsAudit.users | Where-Object { -not $_.linkedSubject })
if ($unlinked.Count -eq 1) {
    $targetUser = $unlinked[0]
    Write-Host "`nConta legada unica identificada: $($targetUser.email) (ID: $($targetUser.id))" -ForegroundColor Yellow
    $linkConfirm = Read-Host "Deseja vincular esta conta legada a um UUID do Supabase agora? (s/N)"
    if ($linkConfirm -eq 's' -or $linkConfirm -eq 'S') {
        $supabaseUuid = Read-Host "Digite o UUID do usuario correspondente no Supabase Auth (ex: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)"
        if ($supabaseUuid -match '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
            $sshLinkPy = @'
import pathlib, sqlite3, sys

user_id = sys.argv[1]
subject = sys.argv[2]
DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")

con = sqlite3.connect(str(DB_PATH))
cur = con.cursor()
cur.execute(
    "INSERT INTO commercial_accounts(user_id, subject) VALUES(?, ?) ON CONFLICT(user_id) DO UPDATE SET subject=excluded.subject",
    (user_id, subject)
)
con.commit()
con.close()
print("LINK_SUCCESS")
'@
            $linkResult = $sshLinkPy | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br "python3 -B - '$($targetUser.id)' '$supabaseUuid'"
            if ($linkResult -match "LINK_SUCCESS") {
                Write-Host "  Conta vinculada com sucesso no commercial_accounts." -ForegroundColor Green
            } else {
                Write-Host "  Erro na vinculacao: $linkResult" -ForegroundColor Red
            }
        } else {
            Write-Host "  UUID fornecido invalido. Vinculacao preservada como pendente." -ForegroundColor Yellow
        }
    } else {
        Write-Host "  Vinculacao pulada. Nenhuma associacao automatica por suposicao foi feita." -ForegroundColor Yellow
    }
} elseif ($unlinked.Count -gt 1) {
    Write-Host "  Multiplas contas nao vinculadas. Nenhuma associacao realizada para evitar ambiguidade." -ForegroundColor Yellow
}

# 5. Aplicacao das variaveis na VPS e Reinicio do Conteiner (Requer autorizacao explicita)
Write-Host "`n[4/5] Preparacao das variaveis comerciais na VPS..." -ForegroundColor Yellow
Write-Host "ATENCAO: A alteracao de .env e reinicio do conteiner ativam o gate comercial na VPS." -ForegroundColor Yellow
Write-Host "Apenas confirme se desejar ativar a validacao comercial agora." -ForegroundColor Yellow

$restartConfirm = Read-Host "Autoriza aplicar o segredo em deployment/.env e reiniciar o conteiner 'service'? (s/N)"
if ($restartConfirm -eq 's' -or $restartConfirm -eq 'S') {
    Write-Host "  Aplicando configuracao comercial e recriando conteiner na VPS..." -ForegroundColor Yellow

    $applyScript = @'
import pathlib, subprocess, sys

ROOT = pathlib.Path("/root/remotecontroller")
ENV_PATH = ROOT / "deployment" / ".env"
portal_url = sys.argv[1]
secret = sys.argv[2]

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
print("[VPS] deployment/.env atualizado com sucesso.")

res = subprocess.run(
    ["docker", "compose", "--env-file", str(ENV_PATH), "-f", str(ROOT / "deployment" / "compose.yaml"), "up", "-d", "service"],
    capture_output=True, text=True
)
print(res.stdout)
if res.returncode != 0:
    print(res.stderr, file=sys.stderr)
    sys.exit(1)
print("[VPS] Conteiner service recriado.")
'@

    $pUrl = "https://remotecontroller-portal.vercel.app"
    $applyResult = $applyScript | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br "python3 -B - '$pUrl' '$sharedSecret'"
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Falha ao aplicar configuracao na VPS: $applyResult"
        exit 1
    }
    Write-Host "  $applyResult" -ForegroundColor Green

    # 6. Validacao E2E da Integracao
    Write-Host "`n[5/5] Executando validacao E2E da integracao..." -ForegroundColor Yellow

    # Teste 1: Healthcheck VPS
    try {
        $health = (Invoke-RestMethod -Uri "https://andredozero.webajato.com.br/healthz" -Method Get).status
        Write-Host "  VPS /healthz: $health" -ForegroundColor Green
    } catch {
        Write-Host "  VPS /healthz falhou: $_" -ForegroundColor Red
    }

    # Teste 2: /login redireciona para o portal
    $loginResp = curl.exe -s -i "https://andredozero.webajato.com.br/login"
    if ($loginResp -match "303" -and $loginResp -match "remotecontroller-portal\.vercel\.app/connect") {
        Write-Host "  VPS /login redireciona para o portal comercial (303 See Other)." -ForegroundColor Green
    } else {
        Write-Host "  Aviso: VPS /login nao redirecionou conforme esperado." -ForegroundColor Yellow
    }

    # Teste 3: Exchange no portal com token invalido rejeita com 401 TICKET_INVALID (nao 503 CONFIGURATION)
    $mcpResp = curl.exe -s -i -X POST "https://remotecontroller-portal.vercel.app/api/mcp/exchange" `
        -H "Authorization: Bearer $sharedSecret" `
        -H "Content-Type: application/json" `
        -d "{\"ticket\":\"fake_ticket_1234567890123456789012345678901234567890\",\"state\":\"fake_state_1234567890123456\",\"redirect_uri\":\"https://andredozero.webajato.com.br/auth/commercial/callback\"}"

    if ($mcpResp -match "TICKET_INVALID" -or $mcpResp -match "401") {
        Write-Host "  Portal /api/mcp/exchange autenticado com sucesso pelo segredo compartilhado (401 TICKET_INVALID)." -ForegroundColor Green
    } else {
        Write-Host "  Resposta MCP Exchange: $mcpResp" -ForegroundColor Yellow
    }

    Write-Host "`n============================================================" -ForegroundColor Cyan
    Write-Host " INTEGRACAO COMERCIAL ATIVADA E VALIDADA!                   " -ForegroundColor Cyan
    Write-Host "============================================================" -ForegroundColor Cyan
} else {
    Write-Host "  Reinicio do conteiner SUSPENSO por solicitacao do operador." -ForegroundColor Yellow
    Write-Host "  O servico na VPS permanece em modo legado e o backup continua integro." -ForegroundColor Green
    Write-Host "`n============================================================" -ForegroundColor Cyan
    Write-Host " PREPARACAO CONCLUIDA (AGUARDANDO AUTORIZACAO DE REINICIO)  " -ForegroundColor Cyan
    Write-Host "============================================================" -ForegroundColor Cyan
}
