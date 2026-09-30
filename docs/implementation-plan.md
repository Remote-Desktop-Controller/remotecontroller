# Plano de implementação

Executar neste workspace novo, conforme autorização integral do anexo.
Usar ciclos de testes de contrato, implementação e gates por bloco.

- [x] 0. Registrar audit e desenho; instalar toolchain necessária.
- [x] 1. domain/src/lib.rs: IDs, estados, política; tests de transições.
- [x] 2. ports/src/lib.rs: repositórios pequenos, Tool, filesystem/process/AST.
- [x] 3. protocol/src/lib.rs e transport/src/lib.rs: envelopes, framing,
  Named Pipes/UDS, autenticação, streaming e retry da mesma operação.
- [x] 4. infrastructure/src/storage.rs e migrations/001_initial.sql:
  operações idempotentes, eventos transacionais, grafo e checkpoints.
- [x] 5. application/src/lib.rs: scheduler com limites por classe,
  execução persistida, cancelamento, deadline e recuperação.
- [x] 6. infrastructure/src/files.rs, processes.rs, tools.rs:
  catálogo extensível, cap-std, escrita atômica, journal/rollback,
  buffers de processo finitos, git e tree-sitter.
- [x] 7. infrastructure/src/context.rs: Moka versionada, rollup e budget.
- [x] 8. apps: composition roots do daemon e gateway rmcp/stdio.
- [x] 9. Testes integração/tempdir, stress e benchmarks Criterion reais.
- [x] 10. README, documentos especializados, dez ADRs, CI e packaging.
- [x] 11. fmt/check/clippy/test/bench, revisar diff e registrar evidências.

Nenhum resultado de teste/benchmark é presumido. Falhas de compilação são
corrigidas antes de avançar; limitações de OS são reportadas explicitamente.
