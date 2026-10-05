# Revisão independente do produto remoto

Data: 2026-10-04. Base declarada: ba5ee3a. Revisão da working tree, incluindo arquivos novos ainda untracked. Revisão inicial estática e leitura de evidência já produzida, sem alterações de produção. Após a revisão, o root autorizou somente o patch de R3 e sua regressão; status abaixo. Nenhum Cargo foi executado pelo revisor.

## Estado após integração pelo root (2026-10-05)

R1/R2 corrigidos e cinco regressões HTTP/IPC passaram; R3 corrigido e regressão de duas sessões passou. R4 foi delimitado como exigência de escritores cooperativos em Unix, conforme security.md; a corrida não é apresentada como eliminada. Clippy estrito e suíte completa Windows passaram. CI tri-OS e release ainda estão em andamento. O veredicto abaixo registra a revisão original, anterior às correções.

## Veredicto original

**Requer correções antes de declarar prontidão técnica do produto remoto.** Não foi identificado P0 nem uma fuga concreta entre contas/dispositivos na superfície examinada. Há dois P1 de disponibilidade/controle no agente e dois P2 de escopo da memória e concorrência externa no filesystem Unix. Os cenários abaixo são deduzidos diretamente do fluxo de código; não se apresentam como testes executados nesta revisão.

A arquitetura implementada corresponde ao spec: serviço de conta/OAuth/relay, conexão de saída do agente, runtime local preservado, aprovação local e histórico durável. A conformidade ainda é parcial pelos achados e pela ausência de evidência final das plataformas/publicação. Um runtime local funcional ou um teste em HTTP loopback não conclui o produto público com ChatGPT.

## Achados concretos

### R1 — P1: cancelamento remoto perde a exceção de concorrência

Locais: `apps/device-agent/src/runner.rs:104-105`; origem do envelope em `apps/remote-service/src/relay.rs:504-507`; tratamento direto no daemon em `crates/infrastructure/src/kernel.rs:195-212`.

O agente reserva passagem para `Request::Cancel`, mas chamadas MCP a `operation.cancel` chegam como `Request::Execute { tool: "operation.cancel", ... }`. Quando quatro chamadas IPC estão ativas, esse comando é ignorado como qualquer execução normal. Ele já foi leased pelo serviço, portanto só volta à fila depois de 30 segundos. `operation.status` sofre a mesma restrição, dificultando a consulta da execução incerta.

Reprodução: iniciar quatro chamadas de longa duração; obter seus operation_ids; chamar `operation.cancel` enquanto os quatro workers permanecem ativos. O comando não alcança o daemon na primeira entrega. Com operações/timeout configurados suficientemente longos, o cancelamento pode ultrapassar o deadline de entrega sem ser executado. Mesmo com timeout padrão, o usuário perde a capacidade de interromper imediatamente essas operações.

Impacto: o produto oferece cancelamento, mas o torna indisponível precisamente sob saturação. Correção esperada: reconhecer as tools de controle no envelope que o relay realmente envia, com capacidade limitada própria, mantendo a concorrência de ferramentas normais limitada. Validar saturação real de quatro workers, cancelamento e consulta de status.

### R2 — P1: resultado expirado bloqueia para sempre a reconexão

Locais: `apps/device-agent/src/runner.rs:48-56,77-83,111-119`; expiração em `apps/remote-service/src/relay.rs:534-537`; lookup de resultado em `apps/remote-service/src/relay.rs:155-156`; falha HTTP em `apps/device-agent/src/http.rs:80`.

`flush` interrompe na primeira falha de POST result e mantém o arquivo da outbox. O loop exige que todo flush tenha sucesso antes de heartbeat e poll. O serviço remove registros relay criados há mais de 24 horas ao aceitar outra chamada para o dispositivo, inclusive registros cujo resultado ainda não foi recebido. Depois dessa remoção, o POST do resultado antigo recebe 404; esse erro é permanente, mas o agente o trata como falha transitória e repete indefinidamente.

Reprodução: concluir uma chamada no daemon, perder conectividade antes de enviar seu resultado, conservar o JSON na outbox; após mais de 24 horas, enviar nova chamada MCP ao mesmo dispositivo, causando limpeza da row antiga; reconectar o agente. O POST do JSON antigo recebe 404, e heartbeat/poll nunca voltam a ocorrer. Para teste automatizado, envelhecer somente created_at no banco de teste produz o mesmo estado sem esperar 24 horas.

Impacto: um computador pareado fica permanentemente offline até intervenção manual na outbox. Correção esperada: distinguir resultado órfão/expirado de falha de rede, preservar sua evidência local em estado explícito e permitir heartbeat/poll e demais entregas. Não apagar silenciosamente o único resultado nem reenviar a operação com outra identidade. Validar reconexão e recuperação do resultado local.

### R3 — P2: ferramentas legadas de histórico ignoram a sessão vinculada

**Status: correção implementada; teste de execução pendente com o root.** Patch em `crates/infrastructure/src/tools.rs:1010-1034`; regressão em `crates/infrastructure/tests/sessions.rs:750`, `bound_recent_and_raw_events_isolate_sessions_and_preserve_legacy_access`. O teste foi escrito antes do patch, mas RED/GREEN não foi executado por esta frente para respeitar a serialização de Cargo. Rustfmt por caminho explícito e `git diff --check` passaram; isso não substitui o teste comportamental.

O patch usa selected_session para herdar Metadata.session_id: recent passa a usar session_events com o limite aplicado na consulta filtrada, e event confere operation_session antes de devolver summary/payload. Chamadas sem sessão conservam recent/raw_event por workspace. A regressão usa duas sessões reais e notas diferentes; valida recent, raw próprio permitido, raw estrangeiro Failed sem payload e os dois caminhos legados. Não foram alterados agente, filesystem/mutations ou terminal.

Locais originais do achado: `crates/infrastructure/src/tools.rs:970-989`; comparação com `selected_session` em `tools.rs:461-479` e os usos em search/compile/notes. A descrição seguinte documenta o comportamento anterior ao patch.

`context.recent` consulta eventos somente pelo workspace. `context.event` consulta payload bruto somente pelo workspace/event_id. Nenhuma delas herda o vínculo da sessão da operação, ao contrário de context.search, context.compile e context.causal. A presença de session_id no envelope portanto não filtra essas duas ferramentas.

Reprodução: criar S1/S2 no mesmo workspace e gravar notas distintas em ambas; executar `context.recent` com Metadata.session_id=S1. Os summaries e event_ids de S2 aparecem. Usar um event_id de S2 em `context.event` ligado a S1 também retorna seu payload.

Impacto: mistura de contexto de trabalhos/chats distintos e bypass do default de continuidade por sessão. Não é acesso cruzado de conta nem sandbox de segurança entre pessoas do mesmo workspace. Correção esperada: aplicar o binding quando ele existe, validar operation_session do evento e manter o comportamento legado workspace-wide quando a chamada não tem sessão. Cobrir essas duas tools no teste de isolamento já existente.

### R4 — P2: identidade final da mutação Unix não permanece fixada até o efeito

Locais: `crates/infrastructure/src/files/mutations.rs:144-170,288-306`.

No Linux/macOS, rename_no_replace compara a identidade do source pelo path e em seguida chama renameat2/renameatx_np pelo nome. Uma troca externa desse componente entre as duas etapas faz o syscall mover outro objeto. O postcheck detecta a divergência depois do efeito, e o rollback corretamente se recusa a tocar a identidade diferente; portanto o arquivo externo permanece movido e o journal fica em conflito. O rollback de mkdir tem gap semelhante entre identity_at/checagem de vazio e remove_dir(name): pode remover um diretório vazio substituto. No Windows, mutation_handle verifica a identidade do handle que recebe a operação, evitando esse intervalo específico.

Reprodução controlada em Unix: pausar a thread após o check de identidade e antes do syscall; outro processo renomeia o source original para outro nome e instala um arquivo diferente no source; retomar. O replacement é movido para destination e a operação termina em conflito. Para rollback de diretório, substituir por outro diretório vazio entre o check e remove_dir. Esta reprodução requer instrumentação de teste ou controle do scheduler; **não foi executada nesta revisão Windows**. O intervalo de corrida é observável no código e não depende de uma API de overwrite.

Impacto: efeitos sobre objeto criado por outro escritor, contrariando a garantia declarada de rollback condicionado à identidade. Confinamento do workspace e no-overwrite do destino continuam presentes; o achado não afirma escape de root. Correção esperada: estabelecer uma estratégia Unix que preserve o objeto efetivamente mutado frente a escritores externos ou limitar explicitamente essa operação até existir essa garantia, e validar a troca concorrente. Testes de conflito antes/depois da operação não cobrem este intervalo.

## Superfícies revisadas sem achado bloqueante adicional

- Contas: Argon2 fora do executor de I/O, limite de trabalhos de senha, tokens aleatórios hashados, cookie HttpOnly/SameSite, CSRF em formulários e mutações autenticadas, rejeição de Origin divergente. JSON de login requer application/json, e o servidor não libera CORS arbitrário. Reset/e-mail ausentes são limite declarado, não uma regressão oculta.
- OAuth: allowlist exata de redirect, PKCE S256, code único/expirado, binding client_id/redirect/resource, scopes na execução/listagem, rotação de refresh transacional e revogação de família em replay. Nenhuma busca de URL arbitrária/CIMD foi encontrada. DCR possui teto global; quotas/rate limits e sua natureza por instância estão documentados.
- Isolamento: resolução de dispositivo por user_id; workspace imutável no heartbeat; resultados por request/device/user; comparação de Metadata completa no serviço; propriedade revalidada na admissão; polling revalida revogação e grant. Não foi encontrada uma via concreta de leitura de outra conta ou postagem de resultado para outro dispositivo.
- Relay/idempotência: operation identity explícita separada de JSON-RPC id, conflito quando payload/sessão mudam, envelope estável no retry, timeout pending e deadline com aviso de execução incerta. Revogação bloqueia entregas futuras; execução já enviada pode continuar e esse limite está declarado.
- Credenciais: perfil fora do workspace, escrita sem substituir destino, diretórios privados, redirect HTTP desabilitado, HTTPS obrigatório com exceção loopback explícita, single-agent lock. Outbox preserva resultados durante falhas transitórias; o limite permanente é R2.
- IPC v2: HMAC com separação server/client, transcript incluindo versão/workspace/nonces, prova do servidor antes de enviar Request, frames de handshake limitados e timeout, secret de 32 bytes sem substituição silenciosa, testes de listener falso/reflection. O servidor retorna metadata solicitada; a autenticação impede o listener falso de obter pedido confiável. Não foi identificada uma falsificação funcional de metadata na superfície revisada.
- Sessões: migração aditiva, workspace binding, claim e estado open transacionais, retry exige sessão igual, closed bloqueia novas admissões, resultado antigo pode ser consultado, search/compile/cache/causal filtrados. R3 restringe a aprovação das ferramentas de histórico completas.
- Terminal: opt-in local com SHA256/programa/argv fixos, capabilities dedicadas, PTY/ConPTY real, writer/capture fora de Tokio/database, input reivindicado pelo Executor antes do efeito, spool durável/limitado, guardian Windows entra no Job antes de criar shell, monitor do pai e kill-on-close. A evidência Windows prova descendente background encerrado. Unix usa sessão/grupos e explicita que não é sandbox para programas que deliberadamente troquem de sessão; isso é escolha legítima para um terminal confiável, não prova de contenção de processos adversariais.
- Filesystem: staging/journal antes da exposição pública, UUID estável, no-replace nativo, preservação de metadados e conflitos, recuperação antes de novas admissões, remove sem remoção recursiva. O achado R4 é específico ao intervalo Unix de identidade; não invalida o resultado nativo Windows existente.

## Evidência observada e limites

Logs lidos diretamente nesta revisão:

- `terminal-final-green.log`: 3 testes nativos Windows passaram; 1 diagnóstico ignorado. Inclui PTY/input retry/resize/close da árvore e spool durável.
- `ipc-transport-green.log`: unitário de transcript, 3 autenticação, 2 framing, 1 mutual-auth e 2 secret-security passaram.
- `runtime-files-sessions-green.log`: 1 integração de mutations e 7 sessões/migração/spool/default search/compile/causal passaram.
- `session-notes-restart-green.log`: 1 teste de decisões/trabalho pendente e não duplicação após restart passou.
- `remote-flow-tests.log`: integração real anterior passou com read/write/session/retry/restart/revoke e 7 testes de segurança do agente. O root informou corretamente que esta evidência antecede IPC v2 e precisa revalidação final integrada.

Não foi executado nenhum gate pelo revisor. Não há aqui prova de CI final Linux/macOS, das races externas Unix, do fluxo final HTTP+agente+IPC v2, instalação/autostart em todos os SOs, carga de produção, HTTPS público, distribuição assinada, publicação/aprovação de catálogo ou execução pela conta real ChatGPT. Os gates automáticos que ainda faltam são engenharia pendente, não ações humanas inevitáveis.

A retirada de CTE após crash é decisão válida e preserva o ADR de causalidade; esta revisão não pede sua reintrodução. Não se exige recuperar o token de pairing depois da entrega única perdida: esse estágio possui limite explícito e requer novo pareamento. Não se apresenta signing/hosting/ChatGPT como concluído por intenção.

## Critério para fechar a revisão

Corrigir R1/R2/R3 e resolver ou delimitar com precisão R4, produzir testes focados desses estados, revalidar fluxo real com IPC v2, concluir gates serializados e CI dos SOs suportados. Depois reler os diffs corrigidos e atualizar este relatório com referências ao código/evidência final. Somente então separar prontidão técnica alcançada das ações externas de conta/domínio/titularidade/assinatura/catálogo/ChatGPT que efetivamente exigem o proprietário.

## R4 — delimitação do modelo Unix

A API POSIX de rename no-replace recebe nome e não aceita condição atômica de inode. A documentação de segurança foi corrigida para exigir escritores cooperativos durante mkdir/move/recovery, incluindo editor/sync acidental. O pós-check detecta após o efeito, não impede mover um substituto. Pode haver conflito pendente que exige reconciliação local; a limitação continua sendo risco de engenharia, não trabalho manual de publicação. Windows permanece com operação por handle/identidade. Não houve reprodução adversarial Unix neste host.
