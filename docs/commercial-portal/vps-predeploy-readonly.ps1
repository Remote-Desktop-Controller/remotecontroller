# Somente leitura: nao abre SQLite, nao cria backup, nao reinicia servicos.
# Senha SSH somente no terminal. Saida filtrada, sem valores secretos.
$vpsPredeployReadOnly = @'
import hashlib, json, os, pathlib, re, shutil, subprocess

ROOT = pathlib.Path("/root/remotecontroller")
PREVIOUS_IMAGE = "sha256:43a6af7fe3750a00d4ebb4eed0bfad542dd6f089b5c0b22e64a037a00031e945"
FILES = [".dockerignore","Cargo.toml","Cargo.lock","deployment/compose.yaml","deployment/Dockerfile",
 "apps/remote-service/Cargo.toml"] + ["apps/remote-service/src/"+n for n in
 ["main.rs","lib.rs","schema.sql","store.rs","commercial.rs","relay.rs","oauth.rs","account.rs","pairing.rs","security.rs","ui.rs"]]

def run(args, cwd=None):
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=30)
        return p.stdout.strip() if p.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None

def parsed(args, cwd=None):
    value = run(args,cwd)
    try: return json.loads(value) if value else None
    except ValueError: return None

def file_hash(path):
    if not path.is_file() or path.is_symlink(): return None
    return hashlib.sha256(path.read_bytes()).hexdigest()

ids = run(["docker","ps","-aq","--filter","label=com.docker.compose.project=deployment",
           "--filter","label=com.docker.compose.service=service"])
if ids is None or len(ids.split()) != 1:
    print(json.dumps({"error":"Expected exactly one production service container; nothing changed"}))
    raise SystemExit(1)
data = parsed(["docker","inspect",ids.strip()])
if not data:
    print(json.dumps({"error":"Container inspect failed; nothing changed"}))
    raise SystemExit(1)
container = data[0]
image = parsed(["docker","image","inspect",container["Image"]])
image = image[0] if image else {}
labels = image.get("Config",{}).get("Labels") or {}
revision = labels.get("org.opencontainers.image.revision","")
revision = revision if re.fullmatch(r"[0-9a-fA-F]{7,40}",revision) else None
binary = run(["docker","exec",container["Id"],"sha256sum","/usr/local/bin/remote-service"])
digest = binary.split()[0] if binary else ""
digest = digest if re.fullmatch(r"[0-9a-f]{64}",digest) else None
head = run(["git","--no-optional-locks","-C",str(ROOT),"rev-parse","HEAD"])
head = head if head and re.fullmatch(r"[0-9a-f]{40}",head) else None
status = run(["git","--no-optional-locks","-C",str(ROOT),"status","--porcelain","--untracked-files=no"])
compose = parsed(["docker","compose","-p","deployment","--env-file",str(ROOT/"deployment/.env"),
                  "-f",str(ROOT/"deployment/compose.yaml"),"config","--format","json"],str(ROOT/"deployment"))
service = (compose or {}).get("services",{}).get("service",{})
effective_env = service.get("environment") or {}
runtime_env = dict(x.split("=",1) for x in container.get("Config",{}).get("Env",[]) if "=" in x)
def commercial(env):
    return {"portalUrlNonempty":bool(env.get("REMOTE_SERVICE_COMMERCIAL_PORTAL_URL")),
            "secretNonempty":bool(env.get("REMOTE_SERVICE_COMMERCIAL_SECRET"))}
module = run(["python3","-B","-c","import sqlite3;print(sqlite3.sqlite_version)"])
db = pathlib.Path("/var/lib/docker/volumes/deployment_service-data/_data/service.db")
disk = shutil.disk_usage(db.parent) if db.parent.is_dir() else None
report = {
 "inspection":"read-only-no-database-open-no-backup-no-restart",
 "container":{"id":container["Id"][:12],"running":container["State"]["Running"],
              "imageId":container["Image"],"matchesPreviousImage":container["Image"]==PREVIOUS_IMAGE},
 "artifact":{"binarySha256":digest,"declaredGitRevision":revision,"created":image.get("Created"),
             "sizeBytes":image.get("Size"),"repoDigests":image.get("RepoDigests",[])},
 "hostCheckout":{"head":head,"trackedChangesPresent":bool(status) if status is not None else None,
                 "criticalFileHashes":{f:file_hash(ROOT/f) for f in FILES}},
 "commercial":{"runningContainer":commercial(runtime_env),"effectiveCompose":commercial(effective_env),
               "effectiveComposeReadOk":compose is not None},
 "compose":{"project":"deployment","path":str(ROOT/"deployment/compose.yaml"),
            "serviceVolumes":service.get("volumes"),"serviceImage":service.get("image"),
            "serviceBuildContext":(service.get("build") or {}).get("context") if isinstance(service.get("build"),dict) else service.get("build"),
            "serviceReadOnly":service.get("read_only"),"serviceStopSignal":service.get("stop_signal"),
            "runtimeEnvironmentNames":sorted(k for k in runtime_env if k.startswith("REMOTE_SERVICE_"))},
 "backupPrerequisites":{"pythonSqliteVersion":module,"freeBytesOnDataFilesystem":disk.free if disk else None,
                       "tools":{k:shutil.which(k) for k in ["gzip","tar","curl","nsenter","python3"]}},
 "configurationFileModes":{},
 "databaseOpened":False,
 "userDeviceSessionCounts":"pending-authorized-consistent-snapshot"
}
for name in [".env","compose.yaml","Caddyfile"]:
    path=ROOT/"deployment"/name
    if path.is_file():
        s=path.stat()
        report["configurationFileModes"][name]={"mode":oct(s.st_mode&0o777),"uid":s.st_uid,"gid":s.st_gid}
print(json.dumps(report,indent=2))
'@
$vpsPredeployReadOnly | ssh -T -o StrictHostKeyChecking=yes -o NumberOfPasswordPrompts=1 -p 22 root@andredozero.webajato.com.br 'python3 -B -'
