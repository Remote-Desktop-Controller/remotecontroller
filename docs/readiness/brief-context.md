# C — Contexto e maintenance

Ler spec e este brief. Ownership: context.rs; novos maintenance.rs e testes
context_maintenance.rs. Não editar storage/ports/tools/config/migrations/apps.
Não executar Cargo builds amplos ou commit; root centraliza gates.

Preservar ContextStoragePort::compile; adicionar API compile_request com task_id,
query/intenção e budget. Relevance concreta: tarefa atual, erros, paths/keywords,
dependências e recent; manter IDs raw, não inventar decisões. Rollups de processos
com status/duração/bytes/exit e refs usando dados reais. UTF-8 budget, cache bounded
e graph_version continuam obrigatórios. Cache key inclui opções de query/tarefa.

Maintenance no LocalStore existente via pub(crate) run(): backup consistente,
retenção conservadora com config tipada e dry-run/report. Proteger journals
prepared e operações ativas/arquivos do workspace. Nunca remover artefatos reais
do usuário. Evitar apagar referências do grafo causando FK inválida. Política
de retenção não pode destruir raw necessário sem backup/auditoria explícita.
Pode começar com limpeza de spools/checkpoints já finalizados e backup antes de
remoção, com confirmação local do CLI feita pelo root.

LocalStore.state_dir/database_path disponíveis. APIs do maintenance devem ser
public wrappers usados pelo CLI root; nenhuma rotina implícita destrutiva ao abrir
kernel. Sugestão: Maintenance::plan/run, BackupInfo, RetentionPolicy com bounds.

Testes: relevância tarefa/query e cache options; budget Unicode; backup reabre,
retenção deixa active/prepared e grafo válido; arquivo do workspace nunca removido.
Escrever report-context.md com APIs/evidências. Não inventar embeddings ou chamar cloud.
