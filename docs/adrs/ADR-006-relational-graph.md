# ADR-006 — Event graph relacional
Status: aceito. event_nodes/event_edges referenciam eventos e operações; CTE
foi substituída por travessia limitada após crash nativo reproduzível em benchmark
Windows GNU/libSQL 0.9.30 com múltiplos eventos. Queries parametrizadas mantêm
grafo, budget e deduplicação sem um banco de grafo separado. Arestas
temporais e causais possuem tipos diferentes. Raw payload e resumo coexistem;
rollup não substitui auditoria. Vector retrieval não é obrigatório.
