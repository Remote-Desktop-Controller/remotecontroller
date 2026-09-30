# B — Processos/policy/durabilidade

Ler spec e este brief. Ownership: processes.rs, processes/platform.rs e novos
submódulos process_*; testes próprios. Não editar apps/config/kernel/storage/ports
ou migrations; pedir interfaces extras ao root. Não rodar builds amplos ou commit.

LocalStore::state_dir() e database_path() já disponíveis. Constructor atual
ProcessManager::new(files,store,rules,buffer_bytes) deve continuar utilizável.
Config extra pode ser método builder ou struct defaults, integrado depois pelo root.

Saída: spool privado por ProcessId, stdout/stderr append+flush/sync, quota finita
por processo/total, tails RAM bounded. Crash deve conservar captura já confirmada.
status/stdout/stderr deve continuar acessível por repository após restart,
informando bytes/truncamento/refs. Não gravar segredos em tracing.

Policy: normalização do executável/argv exatos já implementada; adicionar identity
pinning/fingerprint e rejeitar mudança inesperada; defaults deny. Programas
confiáveis precisam de autorização local explícita; MCP não aprova a si próprio.
Sem sandbox OS disponível, perfil não confinado deve ser declarado e só permitido
com grants atuais de fora/rede; não alegar sandbox. Solicitar ao root código CLI
necessário para guardian/lifecycle/aprovação.

Recovery: reconciliar Starting/Running, preservar output capturado, não matar
PID reutilizado. Provar cancelamento/tree cleanup/crash por OS; propor guardião
Unix implementável no composition root (dois binaries), sem unsafe fork com
heap numa thread Tokio. Sem novos services cloud ou dependência Docker/Node/Python.

Escrever report-processes.md: APIs, migração necessária, evidências e limites;
root integra migrations/guardian/CLI. Não esconder truncamento ou perda de captura.
