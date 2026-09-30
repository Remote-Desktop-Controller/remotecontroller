# ADR-004 — libSQL embedded como SSOT
Status: aceito. SDK oficial libsql 0.9.30, core-only, sem cloud/replicação.
WAL e transações Immediate tornam claims e eventos persistentes. Foi preferido
ao engine Turso novo para reduzir mudança de APIs e risco de engine experimental.
Chamadas locais ficam fora dos workers Tokio. Nenhum banco externo é necessário.
