# C — contexto e maintenance

## APIs integradas

- `context::ContextRequest { workspace, operation, task_id, query, budget }` e
  `ContextEngine::compile_request(request) -> Result<String>`; o `compile`
  anterior permanece compatível e delega para a nova API.
- `EventRepository::task_events(workspace, task, limit)` é implementado pelo root
  em LocalStore com seleção de tarefa, incluindo histórico fora da janela global.
- `Maintenance::new(LocalStore)`, `backup(&Path) -> Result<BackupInfo>`,
  `plan(&RetentionPolicy) -> Result<MaintenanceReport>` e
  `run(&RetentionPolicy, dry_run, Option<&Path>) -> Result<MaintenanceReport>`.
- `RetentionPolicy { older_than_ms: u64, max_records: usize }`: corte absoluto,
  limite entre 1 e 10000, default de 30 dias/100 registros.

## Contexto

Seleção limitada a 256 eventos recentes/da tarefa mais 256 referências causais.
Ranking por tarefa atual, termos da intenção (inclui paths nos summaries e inputs),
erros, referências causais/dependências e recência. Operações são verificadas
contra o workspace. O compilador não gera decisões ou métricas. Rollups usam
JSON realmente persistido, emitidos uma vez por operação: process_id, status,
duration_ms, stdout_bytes, stderr_bytes, exit_code e truncated, quando presentes.
Referências indicam raw_event se o dado veio do evento, raw_operation se veio do
resultado da operação; eventos mantêm seus IDs originais.

Budget máximo 128 KiB; budgets pequenos inclusive zero são respeitados em bytes
UTF-8, sem cortar codepoints. Query limitada a 4096 bytes, 32 termos; entradas
maiores são recusadas, sem colisões artificiais por truncamento. Chave de cache
inclui workspace, graph_version, operação, tarefa, query e budget. Cache TTL
existente preservado; capacidade limitada a 1..512 slots de 132 KiB, cobrando no
mínimo um slot por entrada e peso real quando maior. Limite superior 66 MiB e
512 entradas; entradas acima da capacidade não ficam no cache. Graph version
é verificada novamente antes de devolver qualquer cache/resultado, com cinco
retentativas e Busy sob mutação contínua.

## Backup e retenção

Backup usa `VACUUM INTO` no LocalStore existente, incluindo banco completo,
checkpoints e logs raw; reabre e verifica `PRAGMA quick_check`, depois sync_all.
Spools externos são copiados para `<backup>.spool`, com criação exclusiva,
recusa de symlinks/arquivos especiais e destinos dentro do spool original,
limites de 512 MiB e 10000 arquivos. O CLI precisa segurar daemon.lock e manter
os writers parados para consistência conjunta de banco e spool; a integração
CLI/fence pertence ao root. Nenhuma rotina executa manutenção ao abrir o kernel.

Retenção remove SOMENTE registros de checkpoints `rolled_back` antigos ligados
a operações terminais sem processo ativo Starting/Running e sem checkpoint
prepared na mesma operação. Checkpoints committed permanecem para rollback.
Eventos, edges, processos, raw logs, artifacts e arquivos do workspace nunca são
removidos. Execução exige novo backup explícito e cria auditoria JSON durável
com a intenção antes de apagar. O resultado retornado informa removed_records;
a auditoria prévia registra intenção (zero remoções no momento da gravação),
permitindo recuperar o backup mesmo após crash. Elegibilidade é revalidada em
transação imediata, seguida de foreign_key_check antes do commit.

Backup parcial após falha fica preservado e não habilita remoção; o caller pode
inspecionar e escolher novo destino. GC de spool/raw e de checkpoints committed
fica deliberadamente fora desta política conservadora.

## Evidências

TDD: testes escritos primeiro; root confirmou RED com ContextRequest,
compile_request e maintenance ainda ausentes. Implementação posterior. rustfmt
executado apenas nos arquivos de ownership. Sem Cargo builds amplos ou commits
pelo subagente; gates centralizados pelo root.

`context_maintenance.rs` contém sete casos reais com LocalStore/tempdirs:
query/cache options, tarefa atual, Unicode budgets, backup que reabre com
prepared/workspace preservados, retenção apenas de rolled_back terminal com
raw graph e backup recuperável, rollups com métricas reais e referências,
e cópia de spools sem sobrescrever destinos ou arquivos originais.

GREEN/testes finais pendentes de execução centralizada pelo root.
