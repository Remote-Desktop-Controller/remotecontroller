# ADR-010 — Segurança local-first
Status: aceito. Defaults negam escrita/processos. Root confinement, state-dir
privado fora do workspace, ACL/permissions, segredo IPC e grants explícitos.
Processo externo sem sandbox exige OutsideWorkspaceAccess e NetworkAccess,
além de programa/argumentos exatos. Sem autoelevação, shell genérico ou cloud
obrigatório. Git/parser hostil e mesmo usuário malicioso são riscos documentados.
