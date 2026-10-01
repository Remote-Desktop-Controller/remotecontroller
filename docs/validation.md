# Evidências e limites de validação

Registro histórico v0.1 abaixo. Evidências v0.2: [readiness/progress](readiness/progress.md).

Fechamento local v0.2 em 2026-10-01: 66 testes Windows passaram, zero falhas,
fmt/check/Clippy, release build, oito benchmarks e sete testes MCP/lifecycle/config
com os binários release passaram. Regressões Unix de durabilidade/permissões e
sete casos de configuração passaram em runner Linux com RED/GREEN registrado.
Logs e estado do CI final/publicação estão na matriz de readiness acima.

Ambiente desta sessão: Windows x86_64, Rust 1.98.1 GNU, GCC/MinGW existente.
Repositório novo; nenhum remoto Git configurado. Não há deploy/cloud nesta tarefa.

Execução inicial de `cargo test --workspace -- --nocapture` terminou com código 0:
17 testes passaram. Incluiu domínio (2), filesystem/junction (2), IPC (1),
kernel (7), storage (1), autenticação (1), framing (2), stress (1).
Stress: 10.000 arquivos, 1.000 edições, cancelamento parcial, novo lote,
rollback e restart; todos os 10.000 conteúdos conferidos. Tempo observado
109197 ms dentro do teste, 114.29 s incluindo cleanup. Havia compilação concorrente;
isso é evidência de integridade, não um benchmark representativo de performance.

Execução posterior completa de `cargo test --workspace -- --nocapture`: código 0,
24 testes passaram; um fixture de processo foi ignorado no runner principal e
invocado como filho pelos testes de cancelamento/timeout. Fullstack MCP real
passou com stdout somente protocolo; gateway permaneceu ativo durante restart
abrupto. Segundo fullstack matou daemon durante lote real e confirmou restauração
dos 1.000 arquivos antes de reconnect. Process cancellation, timeout, allowlist,
backpressure, rollback binário e acesso a raw event por referência passaram.
Stress posterior: 179964 ms dentro do teste / 185.28 s incluindo cleanup, durante
compilação e limpeza de caches concorrentes; novamente todos os conteúdos íntegros.

`cargo check --workspace` e Clippy com all-targets/all-features e -D warnings
terminaram com código 0. Clippy corrigiu cinco expressões condicionais equivalentes
no validador. A formatação foi reaplicada depois dessas correções.

Falha ambiental observada: disco C: sem espaço durante build/bench. Foram removidos
somente artifacts Cargo de desenvolvimento (4.7 GiB) e a toolchain stable
duplicada instalada nesta sessão. Profiles dev/test agora desativam debuginfo e
incremental; jobs=2. Nenhum arquivo de projeto do usuário foi removido.

Benchmarks só são reportados após sua execução, na tabela abaixo quando disponíveis.
Uma execução otimizada inicial sofreu STATUS_ACCESS_VIOLATION durante consulta
causal após event_insert; reproduziu no mesmo binário com esse filtro. A consulta
com recursive CTE foi retirada desse caminho e substituída por travessia limitada
com queries parametrizadas. Não se atribui uma causa interna definitiva ao SDK
sem depuração nativa; a nova estratégia precisa passar no benchmark completo.
O benchmark completo da nova estratégia terminou com exit code 0, incluindo os
oito casos. Valores e hardware estão em benchmarking.md. A revisão independente
identificou três erros reproduzidos por testes: cancelamento pós-commit,
case variant da política Windows e fence de recovery depois do lock. Os três
regressions passaram após correção; a revisão de follow-up não apontou pendência
Critical/Important nesses fixes. Também se preserva erro de cleanup em cancelamento.

Validação final: 28 testes passaram, zero falhas; um fixture ignorado no runner
é executado como filho pelos testes. Último stress: 129186 ms, 134.49 s com cleanup.
Log: evidence/tests-final.log. `cargo build --release --workspace` passou.
Os dois testes fullstack também passaram usando target/release via
RUNTIME_TEST_BIN_DIR; log: evidence/release-fullstack.log. Imports de ambos os
executáveis release foram inspecionados com objdump: somente DLLs Windows/UCRT,
sem DLL externa MinGW. Fmt/check/Clippy (-D warnings) passaram no estado final.

Executáveis gerados: local-daemon.exe (13.325.102 bytes) e mcp-gateway.exe
(4.999.454 bytes). Pacote local inclui ambos, documentação, configuração segura
de exemplo e hashes SHA-256; não inclui banco, segredo nem dados do workspace.

Limites conhecidos do v1:

- Execução e testes locais neste host Windows; CI Unix/macOS ainda não executada.
- Sem sandbox OS para processos: regras exatas e grants de rede/fora do root
  são explícitos. Job/process-group são gerenciamento de lifecycle, não sandbox.
- Processos persistem buffer limitado, não spool ilimitado de terminal.
- Readers/writers externos não compartilham o lock de transação do daemon.
- Contexto usa eventos/estado/causalidade e budget UTF-8; ranking por intenção,
  rollups periódicos e SemanticIndex não estão implementados.
- Um workspace ativo por daemon; outros IDs no banco são retomados quando seu
  root é aberto, sem reconciliar journals de um root diferente.
- Sem instalador assinado, atualização automática, retenção/GC ou cloud/licensing.
- Git/parser hostil e ataques por processos do mesmo usuário requerem modelo
  de isolamento mais forte antes de se alegar proteção contra esse atacante.

Última correção: process.spawn canonicaliza o path absoluto recebido antes de comparar com a allowlist. Os três testes de processo passaram usando paths Windows comuns, sem exigir prefixo estendido. Fmt/check/Clippy e rebuild release passaram novamente.
