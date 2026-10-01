# Plano v0.2 — fechamento da entrega

Spec: docs/readiness/spec.md. Executar continuamente por blocos testáveis,
sem recompilar dependências em paralelo desnecessariamente. Branch de integração:
feat/production-readiness; publicar na main após gates e revisão final.

- [x] A. Integridade: FileSnapshot metadata, atomic metadata preservation,
  fingerprints/conflitos, rollback seguro e regressões por OS.
- [x] B. Processos: policy/fingerprints, spool privado com quota, persistência
  incremental e recovery/cleanup de árvore; integração e crash.
- [x] C. Contexto/maintenance: task/query/rollups, retenção configurável/backup
  com integridade do grafo e safety tests.
- [x] D. Integração root: migrations compatíveis, schema e novos contratos,
  Config bounds, permissões por workspace e controle administrativo.
- [x] E. Instalação/bootstrap: CLI init/connect/doctor/install/uninstall/update,
  package por usuário e host config explícito preservando configurações anteriores.
- [x] F. Revisão independente, fullstack SDK e instalação em profile temporário,
  gates, CI tri-OS e release verificável com hashes/documentação.

Ownership: A altera domain/FileSnapshot, files.rs e snapshots em storage.rs.
B altera processes.rs/platform e seus testes; pede interfaces extras ao root.
C altera context.rs/new maintenance.rs e testes, sem tocar migrations/storage.
Root altera apps/protocol/transport/config/kernel/tools/ports/migrations e packaging.
Todas as dependências compartilhadas são integradas pelo root.

## Fechamento retomado em 2026-10-01

Skill writing-plans aplicada ao fechamento do plano existente. Preservar a
branch de integração e as alterações locais; builds Cargo executados em série.

- [x] Validar a regressão `legacy_unprotected_dacl_survives_without_added_parent_aces`
  em `crates/infrastructure/src/files.rs`, comprovar RED sem a correção e GREEN
  com ela. Reexecutar `local-daemon --test fullstack` após rebuild dos binaries.
- [x] Reproduzir `causal_query` em `crates/infrastructure/benches/runtime.rs`
  otimizado; capturar stack nativa com GDB se houver crash, acrescentar regressão
  comportamental de storage e corrigir somente a causa comprovada.
- [x] Executar fmt/check/Clippy, build e testes completos, bench compilation,
  os oito benchmarks, release build e fullstack com binaries release.
- [x] Fazer revisão independente das correções; atualizar evidências,
  `docs/readiness/progress.md`, instalação e benchmarks com resultados reais.
- [x] Commit/push, aguardar CI Windows/Linux/macOS, integrar à main somente com
  gates verdes e gerar/publicar ZIPs v0.2 com manifest e SHA-256 verificáveis.
