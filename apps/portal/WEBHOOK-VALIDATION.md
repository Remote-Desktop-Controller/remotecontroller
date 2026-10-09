# Verificação controlada do webhook — 09/10/2026

Portal: https://remotecontroller-portal.vercel.app
Endpoint: https://remotecontroller-portal.vercel.app/api/asaas/webhook

## Resultado

Validação da configuração, transporte até a Vercel e proteção do banco concluída. **Uma entrega originada pelo Asaas e a idempotência de um pagamento real no Supabase de produção ainda não foram comprovadas.** Não foi criada cobrança real nem inserido pagamento fictício no banco de produção.

## Cadastro consultado diretamente na API Asaas de produção

| Campo | Resultado |
| --- | --- |
| URL do portal | Corresponde ao endpoint esperado |
| Ativo | Sim |
| API | v3 |
| Envio | `NON_SEQUENTIALLY` |
| Fila | Ativa, não interrompida |
| Eventos solicitados | Falta `PAYMENT_PARTIALLY_REFUNDED` |
| Comparação do token cadastrado | Indeterminada: API não devolveu token comparável |

**Ação manual:** acrescentar `PAYMENT_PARTIALLY_REFUNDED` à seleção do webhook, preservando os demais campos. O portal já trata esse evento. Confirmar no painel que o token é o mesmo do arquivo local protegido `.vercel/asaas-webhook-token.txt`; nenhum segredo foi rotacionado ou mostrado nos logs.

## Testes HTTP efetuados pelo operador

Identificador de correlação: `e4e2e3e1-8568-4453-9bb0-ae06ea6ed7e6`.
Origem: script do operador, **não Asaas**.

| Requisição | Resultado |
| --- | --- |
| Sem token | 401 |
| Token incorreto | 401 |
| Token correto e evento controlado sem pagamento | 200 |
| Repetição do mesmo evento controlado sem pagamento | 200 |
| Token correto e pagamento inexistente alegado como recebido | 503, sem concessão de crédito |
| Repetição da mesma alegação de pagamento inexistente | 503, sem concessão de crédito |
| Diagnóstico autenticado de leitura | 200; Supabase e API Asaas acessíveis |

As respostas/IDs de requisição foram correlacionados com os request logs da Vercel: os POSTs 401, 200 e 503 constam no servidor. Isso comprova recepção das requisições do operador. Esses testes não são entregas reais do Asaas e não devem aparecer como tal no histórico de envio do provedor.

## Supabase de produção — leituras antes e depois

| Registro | Antes | Depois |
| --- | --- | --- |
| `portal_orders` | 0 | 0 |
| `portal_entitlements` | 0 | 0 |
| `portal_events` | 0 | 0 |
| `portal_operations` | 0 | 0 |

O bucket técnico `portal_rate_limits`, chave `webhook`, registrou quatro POSTs autenticados na janela nova. Os pedidos sem token/token incorreto foram rejeitados antes desse registro. Eventos controlados sem pagamento e pagamentos não verificados não geraram eventos financeiros, direitos ou ativação.

Repetir um evento ignorado não valida a idempotência positiva do crédito em produção: não existe pedido pago elegível neste banco. Essa distinção é essencial.

## Testes locais e alteração mínima necessária

- 59 testes passaram, incluindo SQL PostgreSQL via PGlite: RECEIVED repetido cria somente um direito; CONFIRMED não libera Pix; login/inspect não ativam; ativação única dura exatamente 2.592.000 segundos; estorno revoga e evento antigo não reativa; RLS isola contas.
- Typecheck/build local e build remoto passaram. Lint: zero erros, três advertências anteriores.
- Acrescentado diagnóstico opcional autenticado `GET /api/asaas/webhook?diagnostics=1`: somente contagens e configuração redigida, sem registros de clientes, valores financeiros, IDs do provedor ou credenciais. O POST e as regras financeiras foram preservados.
- Corrigida uma consulta do diagnóstico para suportar a chave composta de `portal_operations`, sem alteração de tabela/migration.
- Código publicado: `b478050`; deployment READY `dpl_fhCU78AidAjqpqxqqmsjNSR1HPpe`.
- `SALES_ENABLED=false`, ambiente Asaas de produção e `REMOTE_SERVICE_AVAILABLE=false` confirmados por HTTP após os testes. Nenhum DNS/blog/MCP/segredo foi alterado.

As requisições de diagnóstico consultam a API Asaas por GET e o Supabase por leituras; só o rate limit técnico existente é atualizado. Não foram executados POSTs de criação/estorno/cancelamento no Asaas nem RPCs financeiros para fabricar dados de teste em produção.

## Prova ainda pendente

Para comprovar Asaas → portal → ledger com pagamento, é necessário um evento de fato emitido pelo Asaas para um pedido do portal, ou um reenvio legítimo de evento existente. Correlacionar ID do evento no provedor, HTTP 200 no destino, `portal_events.event_key`, um único `portal_entitlements.order_id` e `activated_at IS NULL` enquanto o serviço não estiver liberado. Reenviar o mesmo evento e confirmar que não surge outro direito/período.

Nenhuma cobrança será criada apenas para obter essa prova. Um teste financeiro completo sem dinheiro real deve usar sandbox separado, sem trocar o ambiente de produção atual. Nesta execução, não houve acesso autenticado ao painel de histórico de entregas do Asaas nem foi encontrado um mecanismo documentado para gerar entrega de teste financeiro em produção sem evento; portanto essa origem não foi declarada validada.

Referências: [configuração de webhooks](https://docs.asaas.com/reference/listar-webhooks), [FAQ de entregas/filas](https://docs.asaas.com/docs/webhooks-faq), [autenticação](https://docs.asaas.com/docs/about-webhooks).
