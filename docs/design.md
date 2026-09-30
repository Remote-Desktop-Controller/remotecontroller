# Runtime local — desenho

O pedido anexado é a especificação autorizada. Organizar seis crates coesos:
domain (estados e IDs puros), ports (contratos), application (orquestração),
protocol (DTOs versionados), transport (IPC), infrastructure (adapters locais).
Dois binaries independentes: local-daemon e mcp-gateway. Domain não usa Serde,
Tokio, caminhos do SO ou JSON. Gateway não depende de infrastructure/application.

Escolha: libSQL embedded, sem replicação, em vez de serviços externos ou engine
experimental. MessagePack preserva payloads tipados sem JSON intermediário.
cap-std fornece acesso por directory handles; todas as operações de disco
potencialmente bloqueantes vão a spawn_blocking. Limites por workload e fila
finita precedem a execução. Alterações de arquivos têm journal persistido e
rollback incremental. Banco é a fonte da verdade; contexto usa versão do grafo.

Confiança: usuário local, pasta de estado privada, segredo IPC e ACL/UDS privado.
Process spawn é negado por padrão e exige concessão administrativa explícita;
processos externos não são tratados como sandbox de filesystem. Não há shell
genérico. Cloud não participa do fluxo. Defaults devem falhar fechados.

Teste de aceitação: conexão MCP real, execução daemon, restart/idempotência,
cancelamento/timeout, 10.000 arquivos, lote 1.000, rollback, recuperação,
cache/contexto causal e zero logs no stdout do MCP.
