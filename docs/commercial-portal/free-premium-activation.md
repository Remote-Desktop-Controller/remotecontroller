# Free + Premium: ativação e rollback

## Estado e autorização

Implementação na branch `codex/free-premium`, em checkout separado. O checkout original e as alterações prévias foram preservados. Cinco arquivos de reconciliação/licença administrativa ainda não commitados na origem foram incorporados como pré-requisitos; nenhum dado financeiro foi alterado por isso.

Leitura pública nesta execução confirmou `salesEnabled=false`, `serviceAvailable=true`, preço 990 e origem MCP `https://andredozero.webajato.com.br`. Isso confirma a configuração publicada, não substitui prova de autorização remota. Não alterar esse gate durante a publicação do portal. `FREE_DAILY_ENABLED` permanece false; o caminho existente continua atendendo `/api/mcp/authorize`. Quando a franquia é ativada, esse endpoint legado passa a negar todas as chamadas: serviços antigos não podem executar Free sem reservas.

Não foram autorizados: migration Supabase de produção, reinício/atualização da VPS ou alteração do gate comercial ativo. Esses passos exigem autorização separada conforme o anexo do usuário. Vendas permanecem fechadas; nenhum pagamento real é necessário para testar a franquia.

## Migrations e compatibilidade

Consultar o histórico real de migrations no projeto exclusivo do portal antes de qualquer execução. Não reaplicar scripts já instalados. Ordem para uma instalação ainda na migration inicial:

1. `202610080001_commercial.sql`: base existente; preservar.
2. `202610090001_verified_authorization.sql`: overload aditivo de autorização verificada. O preflight aborta se esse overload já existe: nesse caso auditar a definição instalada, não remover funções/dados para forçar instalação.
3. `202610090002_admin_test_grants.sql`: concessões administrativas separadas de pedidos/receita. Se já instalada, preservar concessões e comparar funções antes de atualizar.
4. `202610090003_daily_free.sql`: novo ledger `portal_usage_operations`/`portal_usage_sessions`, funções de quota, RLS e privilégios. Nova, não executada em produção por esta tarefa.

Somente `service_role` executa as funções de quota; navegador e agente não recebem a chave. `FREE_DAILY_ENABLED=true` requer todas as migrations compatíveis e o novo serviço Rust. Os defaults e deployment seguro mantêm essa variável false.

## Política efetiva

- Free: 1.800 segundos por UUID Supabase/dia em America/Sao_Paulo. União dos intervalos simultâneos, compartilhada por computadores e sessões. Renovação é calculada pelo PostgreSQL; não depende de cron ou de um reset destrutivo.
- Reserva começa no poll que despacha o comando, nunca em login, OAuth, pareamento, heartbeat, catálogo ou inclusão na fila. Janela reservada: até 120s, saldo, meia-noite e validade comercial. A conclusão confirmada usa o relógio SQL, sem aceitar duração informada pelo agente.
- A reserva inteira aparece como indisponível enquanto pendente. Falha do serviço/resultado inconclusivo conserva a reserva até o prazo: isso pode contabilizar tempo reservado sem execução comprovada. É a política explícita para falhas ambíguas, não uma promessa de precisão impossível durante perda de comunicação.
- A sessão lógica termina após 120s sem atividade; a espera ociosa entre operações não é cobrada. IDs de operações concluídas, vencidas ou legadas não ganham nova janela por retry/reconexão/reinício.
- Premium: preço fixo 990 centavos e 2.592.000 segundos a partir do primeiro despacho elegível. Direito pendente verificado pode entrar na fila sem ativar o calendário; reserve ativa. Sem limite diário, sem cobrança automática, sem consumir novas reservas Free. Ao expirar, novas operações voltam ao Free. Licença administrativa mantém seu caminho separado.
- Free recusa operações persistentes `terminal.open`, `terminal.write` e `process.spawn` antes do despacho: o runtime atual não demonstra encerramento com lease desses recursos. Não há promessa de término seguro de operações não interrompíveis. As operações delimitadas têm deadline e cancelamento cooperativo; mutações atômicas já iniciadas podem precisar terminar para preservar integridade.
- Cancelamento/status de operações próprias são recuperação, não autorização para novo trabalho arbitrário. Saldo ou assinatura não verificável falha fechado para novas execuções.

## Sequência de ativação após autorização

1. Preservar o deployment anterior do portal: `https://remotecontroller-portal-5ybp89md0-andrefonns-projects.vercel.app` foi o último Production READY observado antes desta publicação. Registrar também o ID imutável real da imagem Rust atual e salvar a configuração comercial sem expor segredos.
2. Antes de recriar qualquer container, produzir snapshot consistente do libSQL por backup API/VACUUM INTO, verificar integridade e restaurar em ambiente isolado. Preservar os volumes existentes. Os procedimentos já preparados em `docs/commercial-portal` devem ser usados, sem repetir cópia cega de banco WAL.
3. Auditar migrations e executar somente as pendentes autorizadas no Supabase exclusivo. Validar as ACLs/RLS e RPCs por conta de teste administrativa/Free, sem criar pedidos pagos.
4. Distribuir daemon e agente compatíveis com o orçamento monotônico. Não habilitar quota para um daemon antigo que ignore esse orçamento.
5. Publicar a nova imagem Rust preservando usuários, dispositivos, grants, sessões, associações comerciais e volumes. Não recriar identidades por e-mail. O serviço novo usa `/api/mcp/usage`: enquanto a flag estiver false, novas operações falham fechado durante a troca.
6. Ativar `FREE_DAILY_ENABLED=true` no portal e publicar essa configuração. Preservar os segredos e gate já existentes. Manter `SALES_ENABLED=false` durante toda a validação.
7. Validar com conta Free: espera online sem consumo, leitura real, dois dispositivos/sessões, replay, limite e reset; validar conta administrativa existente. Usar sandbox isolado para Premium, nunca dinheiro real. Confirmar o erro `FREE_DAILY_LIMIT_REACHED` e o orçamento local/retorno de recursos.
8. Só abrir vendas após validação final e condições comerciais completas. Ativar vendas é um passo separado.

## Rollback

Rollback operacional restaura o par de versões: portal anterior e imagem Rust anterior, com a configuração comercial anterior. Apenas desligar `FREE_DAILY_ENABLED` com o novo Rust não restaura o funcionamento: ele usa o endpoint novo, que falha fechado com a flag false.

Usar `vercel rollback https://remotecontroller-portal-5ybp89md0-andrefonns-projects.vercel.app --yes` no projeto `remotecontroller-portal` e o compose/imagem imutável preservado para o serviço. Esses comandos são preparados, não executados nesta tarefa. Não remover tabelas/funções de quota nem apagar intervalos durante rollback; migrations aditivas podem permanecer, preservando auditoria e impedindo duplicação de saldo numa reativação no mesmo dia. Restaurar backup de banco somente se houver corrupção comprovada e análise das escritas posteriores, nunca como rotina para trocar binário.

## Limites da evidência

PGlite executa PostgreSQL real em uma conexão, mas não prova contenção entre processos do Supabase hospedado. O E2E usa HTTP local, Auth/PostgREST fixtures e Asaas simulado; não comprova SMTP/Auth hospedados nem eventos originados pelo Asaas. Nenhuma prova local de reinício substitui reinício real da VPS, que depende de autorização. O ledger persistente e a reabertura do store são as provas locais de recuperação.

## Protocolo de despacho e recuperação

O PostgreSQL retorna `lease_remaining_ms`, um orçamento relativo calculado pelo relógio SQL. O serviço desconta o round-trip HTTP e o tempo de espera usando Instant, e o agente desconta o round-trip do poll e a espera até execução. Assim, diferenças entre relógios PostgreSQL/VPS/computador não ampliam a autorização. O daemon precisa anunciar `execution_budget=monotonic_v1`; um daemon antigo é recusado antes da execução.

Um claim exclusivo persistente precede a reserva HTTP. Apenas seu proprietário pode liberar uma reserva por rejeição comprovada antes do despacho, usando `abort`; esse caminho mantém um tombstone do UUID e impede corrida com outro poll. Resultado IPC inconclusivo não é evidência de término: conserva quota. Status/cancelamento têm prioridade e capacidade reservada na fila.

O cálculo de consumo subtrai a união das janelas históricas Premium/Admin dos intervalos Free originais. Assim, uma operação Free em curso deixa de consumir apenas enquanto a licença é válida; revogação/expiração retoma a parcela Free, preservando deadlines e identidades. Pedidos e eventos financeiros permanecem separados. Concessões administrativas excluídas ou reescritas perdem sua isenção histórica de forma conservadora; preserve os registros auditáveis.
