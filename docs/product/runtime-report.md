# Memória operacional do runtime

Escopo Task03, 2026-10-04. Implementação local; execução dos testes coordenada pelo root para não competir pelo target compartilhado. Não constitui validação do produto hospedado/ChatGPT.

## Contratos implementados

- `Metadata.session_id: Option<Uuid>` com default ausente, mantendo protocolo v1 e mensagens antigas.
- Migração 002 aditiva, transacional: `sessions` com UUID, workspace, label e estado open/closed; `operations.session_id` nullable. Bancos v0.2 e operações antigas são preservados. Triggers impedem vínculo cruzado de workspace.
- `Executor::execute_in_session` conserva `execute` sem sessão. Claim, estado open e binding da sessão são verificados na mesma transação Immediate. Retry exige workspace/tool/input/task/session idênticos e pode retornar um resultado já existente mesmo após close; IDs não mudam.
- `session.create` usa operation_id como UUID estável da sessão: crash/retry não cria outra sessão. `session.list` usa cursor/limit; `session.close` bloqueia novas admissões; `session.resume` reabre. Operações já admitidas continuam. Para reabrir uma sessão fechada, chamar resume sem vinculá-lo à própria sessão fechada.
- `context.search` consulta todo o histórico durável, independentemente da janela recent256. Busca literal por summary/type/tool, ordenação seq descendente, cursor exclusivo, filtro session opcional, até 1.000 eventos e 128 KiB de entradas serializadas. Query máximo 4.096 bytes, refs `context.event:<UUID>` preservam acesso aos payloads. Budget zero devolve página vazia e cursor sem perder o evento.
- `process.output`: stdout/stderr, offset absoluto e max_bytes até 128 KiB; encoding hex byte-exato, texto adicional explicitamente utf-8-lossy. Devolve next_offset, capture_state, incomplete/capture_incomplete, stored_bytes, generated_bytes, truncated e raw_ref.
- `process.search`: busca literal de 1..4.096 bytes em janelas de até 8 MiB (default 1 MiB), até 100 hits, contexto até 4.096 bytes de cada lado e budget de até 128 KiB para entradas serializadas. O lookahead inclui matches que começam no fim da janela; next_offset preserva hits omitidos por limit/budget. Captura ativa revisita apenas o sufixo potencialmente incompleto na próxima chamada.
- Cotas de gravação do spool, referências raw e tools stdout/stderr anteriores foram preservadas. Não há CTE recursiva; seleção causal ADR006 permanece intacta.

## Evidência preparada

`crates/infrastructure/tests/sessions.rs` cobre catálogo, lifecycle/restart/isolation/binding, retries fechados, histórico antigo com 320 eventos/pagination/budget/raw refs, migração real de banco 001 com resultado preservado, ranges dentro de UTF-8/quota/restart e mutação repetida sem executar de novo após restart. Testes spool adicionais cobrem match cruzando limite da janela e budget zero. Teste protocol confirma metadata legado sem session_id.

Primeiro GREEN confirmado em `runtime-memory-green.log`: 6 testes de integração passaram, zero falhas. `runtime-spool-green.log`: 3 testes spool passaram, zero falhas. Logs foram lidos diretamente pelo worker; essa rodada antecede o patch de defaults context/session e Task05.

Depois da revisão, `context.search` passou a herdar a sessão da operação; `context.compile/causal` usam `compile_session`, que filtra todas as fontes e inclui sessão na chave de cache. Alvo causal/session override de outra sessão é rejeitado. Chamadas sem sessão preservam seleção legado. Um sétimo teste prova search/compile/cache/causal isolados entre duas sessões e legacy workspace. Validação da revisão segue coordenada pelo root. Rustfmt foi executado nos arquivos editados.

## Self-review e limites explícitos

Sessão é escopo operacional, não memória de treinamento. Não apaga histórico, nem aumenta automaticamente contexto. Cursor não é token de autorização; workspace é sempre verificado separadamente. Os budgets contabilizam entradas serializadas, não o envelope fixo. Busca de histórico usa `lower` do SQLite (case folding ASCII), sem promessa de normalização Unicode. Consulta retornando um único evento maior que o budget conserva o cursor e requer budget maior/raw lookup.

close é fronteira de admissão; não cancela operações em andamento. Session lifecycle aberto/fechado persiste; recovery existente finaliza operações interrompidas e preserva seu vínculo. A prova de produto remoto, daemon/agente real, CI multiplataforma e ChatGPT será feita pela integração root.

## Task05 proposta para revisão root

Terminal em módulo separado `terminal.rs`, com `portable-pty` e ConPTY/PTY nativos. Config `terminal` disabled por default, programa absoluto/argv e SHA256 fixados localmente. Exigir capability dedicada (`AllowInteractiveTerminal`, daemon `--allow-terminal`) mais autorização explícita de efeitos externos/network. Não alterar a rejeição de shells em ProgramRule.

Contrato sugerido: `terminal.open` aceita cwd relativo, rows/cols e nenhuma linha de comando arbitrária; `terminal.read` offset/max_bytes usa spool durável e encoding explícito; `terminal.write` recebe bytes/texto com tamanho limitado; `terminal.resize` valida geometria; `terminal.close` encerra a árvore e finaliza captura. Cada terminal tem UUID/workspace/operation/session quando existente. `write` deve ser reivindicado como operação idempotente antes do efeito; após crash, não repetir input em terminal recriado. Restart marca terminais anteriores interrupted, sem reanexar PIDs.

Risco principal: `portable-pty::spawn_command` pode iniciar o processo antes de associar um Windows Job. Antes da implementação, conferir fonte ConPTY e garantir Job/process-group antes de execução ou usar helper/guardian de ciclo de vida; anexar Job depois do spawn não elimina race de filhos. Fechar shell sozinho não prova encerramento dos descendentes. Readers/writers bloqueantes devem usar workers dedicados/quotas; não prender Tokio nem workers de database. O PTY mistura stdout/stderr; não inventar separação. Shell aprovado possui os privilégios do usuário do SO, não confinamento de arquivo garantido.

Arquivos: `filesystem.mkdir` deve começar com diretório único, pai existente, sem root/.gitignore/.RDC-/symlink/reparse; journal durável antes de create e rollback condicionado à identidade/diretório vazio. `filesystem.move` deve começar com arquivo regular, destino ausente e pais existentes: validação dos dois lados, handles fixados, snapshot/metadata/preconditions, rename same-filesystem, journal before/after e recovery capaz de distinguir original/destino. `filesystem.remove` deve usar caminho explícito; remoção de arquivo usa journal de snapshots existente. Árvores recursivas exigem manifesto completo e cotas de bytes/entradas, ou rejeição explícita até suporte comprovado.

Risco principal de diretórios: `FileSnapshot {content:None}` significa missing, não diretório. Não estender esse campo por convenção; adicionar DTO/variant ou tabela journal específica com kind/path/source/destination/identity/state. Rollback nunca remove diretório que ganhou conteúdo externo nem sobrescreve destino modificado. Testes devem cobrir crash após efeito antes de commit, conflitos externos, symlink/reparse, metadados/ACL e retry sem repetir rename/create.

Referência API primária: [portable-pty 0.9](https://docs.rs/portable-pty/latest/portable_pty/) oferece openpty, spawn_command, try_clone_reader e take_writer; contenção da árvore precisa ser validada na implementação e nos sistemas alvo.
