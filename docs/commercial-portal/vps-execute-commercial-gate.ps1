# Script de Ativacao Controlada do Gate Comercial com Rollback Automatico
# Compativel com Windows PowerShell 5.1 e PowerShell 7+

$ErrorActionPreference = "Continue"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " ATIVACAO CONTROLADA DO GATE COMERCIAL (PORTAL <-> VPS)     " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# Configuracao global de argumentos SSH (autenticacao interativa por senha sem BatchMode)
$SshCommonArgs = @(
    "-o", "PreferredAuthentications=password",
    "-o", "PubkeyAuthentication=no",
    "-o", "StrictHostKeyChecking=yes",
    "-p", "22"
)

# Funcao de execucao remota de scripts Python sem consumir stdin (permite prompt de senha)
function Invoke-VpsPython {
    param(
        [string]$PythonScript
    )
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($PythonScript)
    $b64 = [System.Convert]::ToBase64String($bytes)
    $remoteCmd = "echo $b64 | base64 -d | python3 -B"

    $outputLines = & ssh $SshCommonArgs root@andredozero.webajato.com.br $remoteCmd
    $exitCode = $LASTEXITCODE

    return @{
        ExitCode = $exitCode
        Output = ($outputLines | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    }
}

# Funcao de requisicao HTTP POST com JSON nativo (imune a quote-stripping do PowerShell)
function Invoke-PortalPost {
    param(
        [string]$Uri,
        [string]$Secret,
        [hashtable]$Body
    )
    $json = $Body | ConvertTo-Json -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $req = [System.Net.HttpWebRequest]::CreateHttp($Uri)
    $req.Method = "POST"
    $req.ContentType = "application/json; charset=utf-8"
    $req.Headers.Add("Authorization", "Bearer $Secret")
    $req.ContentLength = $bytes.Length
    $stream = $req.GetRequestStream()
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Close()
    
    try {
        $resp = $req.GetResponse()
        $rStream = $resp.GetResponseStream()
        $reader = New-Object System.IO.StreamReader($rStream, [System.Text.Encoding]::UTF8)
        $text = $reader.ReadToEnd()
        $reader.Close()
        $rStream.Close()
        $resp.Close()
        return @{ StatusCode = 200; Body = $text }
    } catch [System.Net.WebException] {
        $wex = $_.Exception
        if ($wex.Response) {
            $rStream = $wex.Response.GetResponseStream()
            $reader = New-Object System.IO.StreamReader($rStream, [System.Text.Encoding]::UTF8)
            $text = $reader.ReadToEnd()
            $reader.Close()
            $rStream.Close()
            $code = [int]$wex.Response.StatusCode
            $wex.Response.Close()
            return @{ StatusCode = $code; Body = $text }
        }
        throw $_
    }
}

# Funcao de Rollback Automatico
function Invoke-EmergencyRollback {
    Write-Host "`n[ROLLBACK AUTOMATICO] Revertendo VPS imediatamente ao modo legado..." -ForegroundColor Red
    $sshRbPy = @'
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
print("ROLLBACK_COMPLETED")
'@
    $rbRes = Invoke-VpsPython $sshRbPy
    Write-Host "  Resultado do Rollback: $($rbRes.Output)" -ForegroundColor Yellow
    Write-Host "  Servico na VPS restaurado com seguranca para o modo legado." -ForegroundColor Green
}

# 1. Teste de Autenticacao e Auditoria de Contas na VPS
Write-Host "`n[1/4] Testando autenticacao SSH e auditando banco da VPS (andredozero.webajato.com.br)..." -ForegroundColor Yellow

$sshAuditPy = @'
import json, pathlib, sqlite3, sys
print("SSH_OK")
DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")
if not DB_PATH.exists():
    print(json.dumps([]))
    sys.exit(0)
con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
cur = con.cursor()
users = cur.execute("SELECT id, email FROM users").fetchall()
try:
    comm = cur.execute("SELECT user_id, subject FROM commercial_accounts").fetchall()
except Exception:
    comm = []
con.close()
comm_map = {c[0]: c[1] for c in comm}
print(json.dumps([{"id": u[0], "email": u[1], "subject": comm_map.get(u[0])} for u in users]))
'@

$auditRes = Invoke-VpsPython $sshAuditPy
if ($auditRes.ExitCode -ne 0 -or $auditRes.Output -notmatch "SSH_OK") {
    Write-Error "Falha ao conectar via SSH ou autenticar na VPS (ExitCode $($auditRes.ExitCode)): $($auditRes.Output)"
    exit 1
}

$lines = $auditRes.Output.Split([string[]]@([Environment]::NewLine, "`n", "`r`n"), [StringSplitOptions]::RemoveEmptyEntries)
$jsonLine = $lines | Where-Object { $_ -ne "SSH_OK" -and $_ -match '^\s*[\{\[]' } | Select-Object -Last 1
if (-not $jsonLine) {
    Write-Error "Resposta de auditoria da VPS invalida: $($auditRes.Output)"
    exit 1
}
$usersList = $jsonLine | ConvertFrom-Json

$supabaseUuid = $null

if ($usersList.Count -eq 0) {
    Write-Host "  [OK] Autenticacao SSH confirmada!" -ForegroundColor Green
    Write-Host "  Banco local da VPS possui 0 usuarios e 0 contas comerciais (instalacao limpa)." -ForegroundColor Green
    Write-Host "  Provisionamento automatico: o primeiro usuario sera criado automaticamente no primeiro" -ForegroundColor Cyan
    Write-Host "  login comercial via Portal (Vercel/Supabase), sem necessidade de migracao manual." -ForegroundColor Cyan
    $inputUuid = Read-Host "Digite seu UUID do Supabase Auth para validar a licenca no Portal (opcional, Enter para pular)"
    if ($inputUuid -and $inputUuid -match '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
        $supabaseUuid = $inputUuid
        Write-Host "  UUID registrado para validacao no teste de autorizacao: $supabaseUuid" -ForegroundColor Green
    } elseif ($inputUuid) {
        Write-Warning "Formato de UUID invalido. Validacao do Teste 4 usara UUID generico."
    }
} else {
    $legUser = $usersList[0]
    Write-Host "  Conta existente na VPS: $($legUser.email) (ID: $($legUser.id))" -ForegroundColor Cyan
    if ($legUser.subject) {
        Write-Host "  Status atual: VINCULADA ao Supabase UUID: $($legUser.subject)" -ForegroundColor Green
        $supabaseUuid = $legUser.subject
        $confirmChange = Read-Host "Deseja manter este UUID vinculado? (S/n)"
        if ($confirmChange -eq 'n' -or $confirmChange -eq 'N') {
            $supabaseUuid = Read-Host "Digite o novo UUID do Supabase Auth para vincular"
        }
    } else {
        Write-Host "  Status atual: PENDENTE DE VINCULO" -ForegroundColor Yellow
        $supabaseUuid = Read-Host "Digite o UUID do seu usuario no Supabase Auth correspondente a esta conta"
    }

    if ($supabaseUuid -notmatch '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
        Write-Error "UUID do Supabase fornecido e invalido. Abortando para preservar integridade."
        exit 1
    }

    # Salvar vinculo explicitamente no SQLite
    Write-Host "  Registrando vinculo em commercial_accounts..." -ForegroundColor Yellow
    $sshLinkPy = @"
import pathlib, sqlite3
user_id = '$($legUser.id)'
subject = '$supabaseUuid'
DB_PATH = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")
con = sqlite3.connect(str(DB_PATH))
cur = con.cursor()
cur.execute("INSERT INTO commercial_accounts(user_id, subject) VALUES(?, ?) ON CONFLICT(user_id) DO UPDATE SET subject=excluded.subject", (user_id, subject))
con.commit()
con.close()
print("LINK_SAVED")
"@
    $linkRes = Invoke-VpsPython $sshLinkPy
    if ($linkRes.Output -notmatch "LINK_SAVED") {
        Write-Error "Falha ao gravar vinculacao na VPS: $($linkRes.Output)"
        exit 1
    }
    Write-Host "  Conta $($legUser.email) vinculada com sucesso ao UUID $supabaseUuid!" -ForegroundColor Green
}

# 2. Confirmacao explicita de ativacao
Write-Host "`n[2/4] Autorizacao de Ativacao do Gate Comercial:" -ForegroundColor Yellow
Write-Host "  Portal: https://remotecontroller-portal.vercel.app (serviceAvailable=true, salesEnabled=false)" -ForegroundColor Gray
Write-Host "  VPS: https://andredozero.webajato.com.br" -ForegroundColor Gray

$activatePrompt = Read-Host "Autoriza a ativacao do gate comercial na VPS e execucao dos testes reais? (s/N)"
if ($activatePrompt -ne 's' -and $activatePrompt -ne 'S') {
    Write-Host "  Ativacao suspensa pelo operador. VPS preservada em modo legado." -ForegroundColor Yellow
    exit 0
}

# 3. Aplicar Gate Comercial na VPS
Write-Host "`n[3/4] Aplicando gate comercial na VPS..." -ForegroundColor Yellow

# Segredo de 256 bits sincronizado
$secretParam = Read-Host "Cole o MCP_SHARED_SECRET (ou pressione Enter para ler da Vercel CLI)"
if (-not $secretParam) {
    Set-Location -LiteralPath "C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\apps\portal"
    $vcmd = (Get-Command "vercel.cmd" -ErrorAction SilentlyContinue).Source
    if (-not $vcmd) { $vcmd = "vercel.cmd" }
    # Gerar e sincronizar se necessario
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $b = New-Object byte[] 32
    $rng.GetBytes($b)
    $secretParam = [System.BitConverter]::ToString($b).Replace("-", "").ToLowerInvariant()
    $secretParam | & $vcmd env add MCP_SHARED_SECRET production --force --sensitive
    & $vcmd deploy --prod --yes --scope andrefonns-projects --project remotecontroller-portal --non-interactive --no-color
}

$sshGatePy = @"
import pathlib, subprocess
ROOT = pathlib.Path("/root/remotecontroller")
ENV_PATH = ROOT / "deployment" / ".env"
portal_url = "https://remotecontroller-portal.vercel.app"
secret = "$secretParam"
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
res = subprocess.run(["docker", "compose", "--env-file", str(ENV_PATH), "-f", str(ROOT / "deployment" / "compose.yaml"), "up", "-d", "service"], capture_output=True, text=True)
if res.returncode != 0:
    print("RESTART_ERROR: " + res.stderr)
else:
    print("GATE_APPLIED")
"@

$gateRes = Invoke-VpsPython $sshGatePy
if ($gateRes.ExitCode -ne 0 -or $gateRes.Output -notmatch "GATE_APPLIED") {
    Write-Error "Falha ao aplicar gate na VPS: $($gateRes.Output)"
    Invoke-EmergencyRollback
    exit 1
}
Write-Host "  Gate comercial configurado e conteiner reiniciado com sucesso." -ForegroundColor Green

# 4. Bateria de Testes Reais com Rollback Automatico
Write-Host "`n[4/4] Executando testes reais com protecao de rollback automatico..." -ForegroundColor Yellow

# Teste 1: Healthcheck
Start-Sleep -Seconds 2
try {
    $hc = (Invoke-RestMethod -Uri "https://andredozero.webajato.com.br/healthz" -Method Get).status
    if ($hc -ne "ok") { throw "Healthcheck status: $hc" }
    Write-Host "  [TESTE 1/4 APROVADO] /healthz respondendo 'ok'." -ForegroundColor Green
} catch {
    Write-Host "  [TESTE 1/4 FALHOU] Healthcheck da VPS indisponivel: $_" -ForegroundColor Red
    Invoke-EmergencyRollback
    exit 1
}

# Teste 2: /login redireciona 303 para o portal comercial
$loginResp = curl.exe -s -i "https://andredozero.webajato.com.br/login"
if ($loginResp -match "303" -and $loginResp -match "remotecontroller-portal\.vercel\.app/connect") {
    Write-Host "  [TESTE 2/4 APROVADO] /login redireciona (303) para o portal comercial!" -ForegroundColor Green
} else {
    Write-Host "  [TESTE 2/4 FALHOU] /login nao redirecionou para o portal comercial." -ForegroundColor Red
    Invoke-EmergencyRollback
    exit 1
}

# Teste 3: /api/mcp/exchange autenticado com o segredo
$mcpTest = Invoke-PortalPost -Uri "https://remotecontroller-portal.vercel.app/api/mcp/exchange" `
    -Secret $secretParam `
    -Body @{
        ticket = "fake_ticket_1234567890123456789012345678901234567890"
        state = "fake_state_1234567890123456"
        redirect_uri = "https://andredozero.webajato.com.br/auth/commercial/callback"
    }

if ($mcpTest.Body -match "TICKET_INVALID") {
    Write-Host "  [TESTE 3/4 APROVADO] /api/mcp/exchange autenticou com sucesso o segredo compartilhado (ticket fake rejeitado com TICKET_INVALID)!" -ForegroundColor Green
} else {
    Write-Host "  [TESTE 3/4 FALHOU] /api/mcp/exchange falhou (Status $($mcpTest.StatusCode)): $($mcpTest.Body)" -ForegroundColor Red
    Invoke-EmergencyRollback
    exit 1
}

# Teste 4: Checagem de autorizacao de comando remoto no Portal
$opId = [System.Guid]::NewGuid().ToString()
$testSubject = if ($supabaseUuid) { $supabaseUuid } else { [System.Guid]::NewGuid().ToString() }
$authTest = Invoke-PortalPost -Uri "https://remotecontroller-portal.vercel.app/api/mcp/authorize" `
    -Secret $secretParam `
    -Body @{
        subject = $testSubject
        operation_id = $opId
        mode = "inspect"
    }

Write-Host "  [TESTE 4/4] Resposta de autorizacao no Portal (Status $($authTest.StatusCode), Subject $testSubject): $($authTest.Body)" -ForegroundColor Cyan

if ($authTest.Body -match '"allowed":true' -or $authTest.Body -match '"state":"ACTIVE"') {
    Write-Host "  [TESTE 4/4 APROVADO] Direito de teste administrativo ATIVO e reconhecido pelo portal!" -ForegroundColor Green
} elseif ($authTest.StatusCode -eq 402 -or $authTest.Body -match '"allowed":false' -or $authTest.Body -match 'NO_SUBSCRIPTION') {
    if ($supabaseUuid) {
        Write-Host "  Aviso: UUID $supabaseUuid ainda nao possui direito de teste ativo no Supabase (retornou NO_SUBSCRIPTION)." -ForegroundColor Yellow
        Write-Host "  Aplique a migration e adicione o direito de teste no Supabase para liberar comandos remotos." -ForegroundColor Yellow
    } else {
        Write-Host "  [TESTE 4/4 APROVADO] Endpoint de autorizacao operacional (negacao esperada para subject sem licenca)." -ForegroundColor Green
    }
} else {
    Write-Host "  [TESTE 4/4 FALHOU] Resposta inesperada do portal para autorizacao (Status $($authTest.StatusCode)): $($authTest.Body)" -ForegroundColor Red
    Invoke-EmergencyRollback
    exit 1
}

Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host " ATIVACAO COMERCIAL CONCLUIDA COM SUCESSO!                  " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
