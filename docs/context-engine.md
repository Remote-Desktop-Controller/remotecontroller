# Contexto e memória causal

Histórico raw e grafo ficam no banco. recent projeta uma janela limitada;
causal usa travessia limitada de predecessores com queries parametrizadas e deduplicação.
FollowedBy representa ordem temporal; não demonstra dependência semântica.
CausedBy/Mutated identificam eventos/resultados da operação. Artefatos vinculam
paths à operação que os produziu. Não há inferência de causalidade por embedding.

As queries de projeção não carregam raw BLOBs: `context.event` recupera um evento
raw por ID quando o host precisa auditar a referência. Travessia possui limite interno
de nós para evitar percorrer história ilimitada por uma consulta pequena.
ContextCompiler é função pura: estado/version header; prioridade para erros,
timeouts, arquivos alterados e decisões; depois eventos recentes. Cada linha
preserva event_id e operation_id para consulta raw. O budget é **bytes UTF-8**,
entre 0 e 131072, não uma estimativa fictícia de tokens. Truncamento respeita
fronteiras de caractere. Terminal extenso não é incluído no contexto.

Rollups usam resumo, status e tamanho com referência raw. Logs/resultados
operacionais completos até os limites de captura permanecem locais. O compiler
nunca substitui nem apaga raw events. A v0.2 aceita task_id e query para ranking
por palavras, paths e prioridade da tarefa atual; erros e métricas reais de
processos produzem rollups com referências raw. Embeddings/cloud são opcionais.

Moka é bounded por capacidade, TTL configurável (default 3 h). A chave contém
workspace_id, graph_version, operation/task/query selecionadas e budget. Workspace version
é relida depois da compilação/cache hit; se mudar, o engine tenta novamente.
Após cinco alterações concorrentes retorna Busy em vez de contexto obsoleto.
Entradas de versões antigas expiram; não podem satisfazer chave nova.

Cache hit/miss possuem contadores. Nenhuma operação, checkpoint ou raw event tem
única cópia no cache. Reiniciar o daemon começa cache vazio e lê SSOT.
