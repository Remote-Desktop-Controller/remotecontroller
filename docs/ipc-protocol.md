# IPC v2

Named Pipes no Windows e UDS no Unix. Frames MessagePack têm prefixo u32 big-endian; dados até 8 MiB, handshake até 1 KiB, rejeitados antes de alocar. A troca inteira de autenticação tem deadline de cinco segundos.

1. ClientHello: versão 2 e nonce aleatório de 32 bytes.
2. ServerChallenge: workspace, nonce aleatório e HMAC-SHA256 do transcript com domínio `rdc-ipc/v2/server`.
3. Cliente verifica a prova do servidor antes de enviar ClientProof, HMAC com domínio separado `rdc-ipc/v2/client`.
4. Servidor verifica e envia Welcome consistente com o workspace autenticado.

O segredo privado de 32 bytes vem do CSPRNG do sistema, nunca atravessa o transporte, não aparece em Debug/tracing e não é regenerado silenciosamente. Provas incluem versão, workspace e ambos os nonces; comparação é constante. Legacy Hello recebe VersionMismatch: atualize daemon, gateway e agente juntos. Não há downgrade.

Metadata inclui protocol_version, request_id, trace_id, workspace_id, task_id, session_id, operation_id e deadline (epoch ms). Request suporta Catalog, Heartbeat, Execute, Cancel, GetOperation; Response contém metadata e Result tipado. request_id correlaciona; operation_id identifica efeito idempotente; session_id filtra continuidade. Deadline usa o relógio compartilhado da máquina.

Uma conexão trata autenticação, um pedido, eventos advisory limitados e resposta. Chamadas concorrentes usam conexões independentes. Falhas de I/O permitem repetir o mesmo envelope dentro do deadline; falhas de autenticação/versão não são retentadas como indisponibilidade. Desconexão não desfaz efeito persistido: consulte operation.status antes de repetir uma mutação com outra identidade.

Banco/secret/socket ficam fora do workspace. Unix exige proprietário e permissões privadas; Windows rejeita reparse point no secret e usa DACL protegida. Instaladores distribuem componentes da mesma versão.
