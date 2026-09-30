# ADR-009 — Escritas atômicas e journal de lote
Status: aceito. Temporário no mesmo parent, create_new, write, sync_all,
rename/replace e verify. Checkpoint incremental precede a primeira alteração.
Lote faz rollback integral quando falha; journal persiste para crash recovery.
Não prometer atomicidade de todos os arquivos perante writers externos.
