# Plano v0.2 — fechamento da entrega

Spec: docs/readiness/spec.md. Executar continuamente por blocos testáveis,
sem recompilar dependências em paralelo desnecessariamente. Branch de integração:
feat/production-readiness; publicar na main após gates e revisão final.

- [ ] A. Integridade: FileSnapshot metadata, atomic metadata preservation,
  fingerprints/conflitos, rollback seguro e regressões por OS.
- [ ] B. Processos: policy/fingerprints, spool privado com quota, persistência
  incremental e recovery/cleanup de árvore; integração e crash.
- [ ] C. Contexto/maintenance: task/query/rollups, retenção configurável/backup
  com integridade do grafo e safety tests.
- [ ] D. Integração root: migrations compatíveis, schema e novos contratos,
  Config bounds, permissões por workspace e controle administrativo.
- [ ] E. Instalação/bootstrap: CLI init/connect/doctor/install/uninstall/update,
  package por usuário e host config explícito preservando configurações anteriores.
- [ ] F. Revisão independente, fullstack SDK e instalação em profile temporário,
  gates, CI tri-OS e release verificável com hashes/documentação.

Ownership: A altera domain/FileSnapshot, files.rs e snapshots em storage.rs.
B altera processes.rs/platform e seus testes; pede interfaces extras ao root.
C altera context.rs/new maintenance.rs e testes, sem tocar migrations/storage.
Root altera apps/protocol/transport/config/kernel/tools/ports/migrations e packaging.
Todas as dependências compartilhadas são integradas pelo root.
