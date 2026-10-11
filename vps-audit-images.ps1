#requires -Version 5.1
<#
.SYNOPSIS
    Inspeção estritamente somente-leitura dos IDs das imagens Docker na VPS.
#>
param(
    [string]$HostName = "andredozero.webajato.com.br",
    [string]$User = "root",
    [int]$Port = 22
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   AUDITORIA DAS IMAGENS DOCKER NA VPS (READ-ONLY)        " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$pythonPayload = @'
import json, subprocess, sys

def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()

print("\n--- 1. CONTÊINER EM EXECUÇÃO ---")
cid = run(["docker", "ps", "-q", "--filter", "label=com.docker.compose.project=deployment", "--filter", "label=com.docker.compose.service=service"])
if not cid:
    print("ERRO: Contêiner não encontrado.")
    sys.exit(1)

inspect = json.loads(run(["docker", "inspect", cid]))[0]
running_img_id = inspect["Image"]
running_created = inspect["Created"]
print(f"ID do Contêiner: {cid[:12]}")
print(f"Status: {inspect['State']['Status']}")
print(f"Iniciado em: {inspect['State']['StartedAt']}")
print(f"Image ID em execução: {running_img_id}")

try:
    running_bin_sha = run(["docker", "exec", cid, "sha256sum", "/usr/local/bin/remote-service"]).split()[0]
    print(f"SHA256 do binário em execução: {running_bin_sha}")
except Exception as e:
    print(f"Erro lendo binário do contêiner: {e}")

print("\n--- 2. TAG DE ROLLBACK (remote-service:rollback-precommercial) ---")
rb_img_id = run(["docker", "images", "-q", "remote-service:rollback-precommercial"])
print(f"Image ID da tag de rollback: {rb_img_id}")
if rb_img_id:
    try:
        rb_bin_sha = run(["docker", "run", "--rm", "remote-service:rollback-precommercial", "sha256sum", "/usr/local/bin/remote-service"]).split()[0]
        print(f"SHA256 do binário do rollback: {rb_bin_sha}")
    except Exception as e:
        print(f"Erro lendo binário do rollback: {e}")

print("\n--- 3. IMAGEM RECÉM-COMPILADA (deployment-service ou service) ---")
new_img_id = run(["docker", "images", "-q", "deployment-service:latest"])
if not new_img_id:
    new_img_id = run(["docker", "compose", "-f", "/root/remotecontroller/deployment/compose.yaml", "images", "-q", "service"])
print(f"Image ID da nova imagem: {new_img_id}")
if new_img_id:
    try:
        new_bin_sha = run(["docker", "run", "--rm", new_img_id, "sha256sum", "/usr/local/bin/remote-service"]).split()[0]
        print(f"SHA256 do binário novo compilado: {new_bin_sha}")
    except Exception as e:
        print(f"Erro lendo binário novo: {e}")

print("\n--- 4. TODAS AS IMAGENS RELACIONADAS ---")
imgs_table = run(["docker", "images", "--filter", "reference=*remote-service*", "--filter", "reference=*deployment*"])
print(imgs_table)

print("\n--- 5. DIAGNÓSTICO DE SEGURANÇA DE ROLLBACK ---")
if running_img_id == new_img_id:
    print("AVISO: O Image ID do contêiner em execução é IGUAL ao da nova imagem!")
else:
    print("OK: O contêiner em execução possui Image ID DIFERENTE da nova imagem.")

if rb_img_id and rb_img_id == running_img_id:
    print("OK: A tag de rollback aponta EXATAMENTE para a imagem do contêiner atualmente em execução.")
else:
    print("ALERTA: A tag de rollback NÃO aponta para a imagem do contêiner em execução!")
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
