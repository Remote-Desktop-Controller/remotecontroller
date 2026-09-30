# IPC v1

Transporte local: Named Pipes no Windows, UDS no Unix. Não há TCP. O primeiro
frame é `Hello {protocol_version,secret}`. O servidor compara o segredo em tempo
constante e responde `Welcome {protocol_version,workspace_id}` ou erro. Versão
incompatível é rejeitada antes da execução.

Cada frame tem **u32 big-endian de tamanho + MessagePack**. Tamanho máximo:
8 MiB; tamanho zero ou superior ao limite é rejeitado antes da alocação do body.
São DTOs Serde tipados; JSON existe somente nos argumentos/resultados MCP e nas
representações locais estruturadas. Não há cadeia JSON/String/JSON no transporte.

| Envelope | Dados |
|---|---|
| Metadata | protocol_version, request_id, trace_id, workspace_id, task_id, operation_id, deadline |
| Request | Catalog, Heartbeat, Execute, Cancel, GetOperation |
| Response | Metadata e Result<Value,ErrorEnvelope> |
| Event | Metadata, completed, total, message |
| Error | code tipado, message, retryable |

Deadline é Unix epoch em milissegundos; clocks dos dois processos são da mesma
máquina. IDs são UUIDs no wire e newtypes no domínio. Execute exige workspace_id.
O request_id correlaciona resposta; operation_id identifica efeito idempotente.

Uma conexão trata handshake + um request + zero ou mais eventos + resposta.
Chamadas concorrentes usam conexões independentes. O client reconecta com o mesmo
envelope até quatro tentativas em falhas de I/O, dentro do deadline.
Cancel/GetOperation e Heartbeat podem usar outra conexão enquanto uma tool roda.

Handshake e recepção inicial têm limite de 5 s. Progress é advisory, limitado e
coalescível; mensagens excedentes podem ser descartadas. Writes para cliente
lento têm timeout. A operação persiste independentemente da conexão, logo
desconectar durante lote não aborta seu journal. O host consulta status/retry.

Erros distinguem versão, autenticação, política, busy, conflito, cancelamento,
timeout, armazenamento e execução. Nunca transmitir segredo em tracing.
