# Entrega do runtime local — Windows v0.1.0

Arquitetura implementada: seis crates de biblioteca e dois apps nativos,
gateway MCP separado do daemon por Named Pipe/UDS. Domain puro com IDs/estados,
ports pequenos, application/scheduler e adapters de infraestrutura. Mermaid em
architecture.md; árvore e execução em README.md. Não havia código anterior.

27 tools: filesystem read/read_range/list/search/stat/write/patch/batch_edit;
process spawn/status/stdout/stderr/kill; git status/diff/show; workspace info;
context recent/causal/compile/event; operation status/cancel; checkpoint
create/rollback; code inspect/symbols (Rust/Python, tree-sitter).

IPC v1: MessagePack, framing u32, máximo 8 MiB, handshake/segredo local,
request/response/progress, heartbeat, cancelamento, reconnect e retry idempotente.
Gateway usa rmcp stdio; stdout é exclusivamente protocolo, logs em stderr.

Storage: libSQL embedded local, WAL, synchronous FULL, migrations e índices.
workspaces/tasks/operations/tool_calls/events/event_nodes/event_edges/artifacts/
checkpoints/processes. Operação/resultados/eventos/versionamento são write-through.
Grafo causal usa travessia parametrizada e limitada; CTE foi substituída após
crash nativo reproduzido em benchmark otimizado. Raw events permanecem acessíveis
por referência; projeções não carregam BLOBs de terminal.

Cache: Moka bounded + TTL, chave com graph_version/operation/budget; versão
relida depois de hit/compilação. Cache não é SSOT. Context compiler prioriza
erros/mutações/recentes e mantém IDs para auditoria, com budget UTF-8.

Scheduler: admissão finita, semaphores por workload, mpsc finito para progress,
JoinSet de conexões e cancelamento cooperativo. CPU/C/IO bloqueante usa
spawn_blocking. Control-plane status/cancel permanece disponível com fila cheia.
Métricas cobrem fila, ativos, duração, sucesso/falha, cancelamento, cache e storage.

Segurança: root confinement por cap-std, traversal/links/junctions/ADS/reserved
paths/exclusões/gitignore, limites de dados, state-dir privado fora do workspace,
ACL Windows e permissions Unix, segredo IPC, allowlist de executável/args e
grants explícitos. Sem shell genérico, autoelevação ou cloud no caminho crítico.

Escritas: temporário, sync, atomic replace, verify; lotes com snapshot incremental,
journal antes da primeira alteração, rollback e artifacts persistidos. Poison
fence sob lock impede mutação antes de recovery. Cancelamento tardio conserva
sucesso/resultado depois de commit; erro de cleanup não é ocultado.

Recovery: journals prepared restaurados antes da admissão; operações interrompidas
terminam Failed com motivo; retry de ID já processado não repete efeito. Windows
Job Objects encerram filhos no cancelamento/crash; Unix process groups ficam
isolados no adapter. PIDs persistidos não são mortos cegamente depois de restart.

Verificação: fmt, check, Clippy all-targets/all-features -D warnings, 28 testes,
bench compilation e oito benchmarks reais; build release e fullstack com release.
Stress 10.000 arquivos/1.000 edições/cancelamento/rollback/restart conferiu todos
os conteúdos. Logs e valores: validation.md e benchmarking.md.

Distribuição: binários Windows e ZIP local; CI matrix Linux/Windows/macOS definida,
ainda sem execução remota. Não existe remoto Git nesta pasta; entrega é local.

Limitações reais: nenhuma sandbox OS completa; filhos requerem grants explícitos
de fora do root e rede. Buffer de processos limitado; snapshots restauram conteúdo
e existência, sem promessa de preservar todos os metadados/ACLs. Writers externos
precisam de coordenação. Um workspace ativo por daemon. Ranking por intenção,
rollups periódicos, retention/GC, instalador assinado e updater não implementados.
Não se declara validação Linux/macOS nem isolamento contra malware do mesmo usuário.

Próximos passos necessários para distribuição ampla: executar CI em outros OS,
validar isolamento/process lifecycle Unix, ampliar preservação de metadados e
política de output/retention, assinar/notarizar instaladores. Vector retrieval e
cloud/auth/licensing permanecem opcionais e fora do núcleo offline.
