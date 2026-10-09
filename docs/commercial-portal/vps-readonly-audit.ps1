# Executar manualmente no PowerShell. Pede a senha no terminal, nunca no chat.
# Apenas inventario Docker e stat de arquivos; nao abre o banco nem imprime segredos.
$vpsAuditReadOnly = @'
import json, os, posixpath, shutil, subprocess
from urllib.parse import urlsplit

def read(cmd):
    p = subprocess.run(cmd, capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else None

def safe(key, value):
    if "URL" in key or key == "REMOTE_SERVICE_REDIRECT_URIS":
        out = []
        for item in value.split(","):
            try:
                u = urlsplit(item.strip())
                public_path = u.path if key == "REMOTE_SERVICE_REDIRECT_URIS" and u.path in ["/connector_platform_oauth_redirect","/auth/commercial/callback"] else ""
                out.append(u.scheme + "://" + (u.hostname or "") + ((":" + str(u.port)) if u.port else "") + public_path)
            except ValueError:
                out.append("<invalid-url-redacted>")
        return ",".join(out)
    return "<redacted>" if "://" in value or "?" in value else value

if not shutil.which("docker"):
    print(json.dumps({"error": "Docker not found; nothing changed"}))
    raise SystemExit(1)

report = {"tools": {k: shutil.which(k) for k in ["docker","python3","sqlite3","restic","git"]},
          "composeVersion": read(["docker","compose","version","--short"]), "containers": []}
ids = read(["docker","ps","-aq"])
if ids is None:
    print(json.dumps({"error": "Docker unavailable or permission denied; nothing changed"}))
    raise SystemExit(1)

allowed = ["REMOTE_SERVICE_PUBLIC_URL","REMOTE_SERVICE_DATABASE","REMOTE_SERVICE_BIND",
           "REMOTE_SERVICE_REDIRECT_URIS","REMOTE_SERVICE_ALLOW_HTTP_LOOPBACK",
           "REMOTE_SERVICE_COMMERCIAL_PORTAL_URL"]
cli = {"--public-url": allowed[0], "--database": allowed[1], "--bind": allowed[2],
       "--redirect-uris": allowed[3], "--commercial-portal-url": allowed[5]}
for cid in ids.split():
    raw = read(["docker","inspect",cid])
    if raw is None:
        continue
    d = json.loads(raw)[0]
    cfg = d.get("Config") or {}
    labels = cfg.get("Labels") or {}
    env = dict(v.split("=",1) for v in cfg.get("Env",[]) if "=" in v)
    args = d.get("Args") or []
    candidate = any(k.startswith("REMOTE_SERVICE_") for k in env) or "remote-service" in d.get("Path","")
    item = {"id": d["Id"][:12], "name": d["Name"], "image": cfg.get("Image"),
            "imageId": d.get("Image"), "running": d["State"]["Running"],
            "status": d["State"]["Status"], "restartCount": d.get("RestartCount"),
            "compose": {k: labels.get("com.docker.compose."+k) for k in
                        ["project","service","project.working_dir","project.config_files","project.environment_file"]},
            "ports": (d.get("NetworkSettings") or {}).get("Ports"),
            "mounts": [{k: m.get(k) for k in ["Type","Name","Source","Destination","RW"]}
                       for m in d.get("Mounts",[])], "remoteServiceCandidate": candidate}
    if candidate:
        effective = {k: env[k] for k in allowed if k in env}
        for i, arg in enumerate(args):
            flag, sep, val = arg.partition("=")
            if flag in cli:
                effective[cli[flag]] = val if sep else (args[i+1] if i+1 < len(args) else "")
        item["safeConfiguration"] = {k: safe(k,v) for k,v in effective.items()}
        item["commercialSecretPresent"] = bool(env.get("REMOTE_SERVICE_COMMERCIAL_SECRET")) or any(a.split("=")[0] == "--commercial-secret" for a in args)
        item["commercialSecretEnvAtLeast32"] = len(env.get("REMOTE_SERVICE_COMMERCIAL_SECRET","")) >= 32
        item["databaseFiles"] = []
        dbpath = effective.get("REMOTE_SERVICE_DATABASE","")
        if dbpath.startswith("/"):
            dbpath = posixpath.normpath(dbpath)
            for m in sorted(d.get("Mounts",[]), key=lambda x: len(x["Destination"]), reverse=True):
                dest = m["Destination"].rstrip("/")
                if dbpath == dest or dbpath.startswith(dest+"/"):
                    host = os.path.join(m["Source"],dbpath[len(dest):].lstrip("/"))
                    for suffix in ["","-wal","-shm"]:
                        path = host + suffix
                        if os.path.isfile(path):
                            s = os.stat(path)
                            item["databaseFiles"].append({"path": path, "bytes": s.st_size,
                                "uid": s.st_uid, "gid": s.st_gid, "mode": oct(s.st_mode & 0o777)})
                    break
        item["databaseOpened"] = False
    report["containers"].append(item)
print(json.dumps(report, indent=2))
'@
$vpsAuditReadOnly | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'

