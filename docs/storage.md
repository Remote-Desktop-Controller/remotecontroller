# Persistência local

libSQL oficial, feature `core` apenas. `Builder::new_local` abre runtime.db;
WAL, synchronous=FULL, foreign_keys=ON e busy_timeout=5 s são configurados.
Sem rede/replicação. Calls embedded ficam em spawn_blocking, com pool lógico
de quatro acessos. Prepared queries usam parâmetros, nunca SQL do host.

Migration 001 é idempotente e versionada em schema_migrations. Tabelas:
workspaces, tasks, operations, tool_calls, events, event_nodes, event_edges,
artifacts, checkpoints e processes. Índices cobrem workspace/status, timeline,
operação, arestas de entrada, artefatos e checkpoints pendentes.

claim usa INSERT OR IGNORE em transação Immediate, cria ToolCall, evento queued,
node e relação temporal com o anterior. finish valida transição, grava output,
erro, evento terminal e aresta causal em uma transação, atualizando graph_version.
Persistir vem antes de observar sucesso ou popular cache.

Lote: checkpoint preparado primeiro; arquivos são substituídos; uma transação
grava artifacts, evento mutated, aresta, graph version e marca journal committed.
Crash antes desse commit restaura snapshots. Crash depois dele conserva edições
integrais e registra operação interrompida caso resultado terminal não exista.

Payloads raw são BLOBs, resultados/inputs serializados localmente em JSON.
Snapshots registram bytes + path + existência anterior. Não há cópia integral
do workspace. Limites de bytes/arquivos precedem alocação/aplicação massiva.

O schema v1 não implementa retenção/GC automática. Histórico e checkpoints
continuam disponíveis até intervenção explícita do operador; monitore o volume
da pasta privada. Não editar o banco enquanto o daemon estiver ativo.
