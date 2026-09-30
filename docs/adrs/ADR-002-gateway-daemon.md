# ADR-002 — Gateway separado do daemon
Status: aceito. Dois processos separam lifecycle MCP da execução durável.
Gateway depende somente de protocolo/transporte e rmcp. O daemon conhece registry
e adapters. Desconexão não interrompe transação de arquivos. Alternativa rejeitada:
executar tools dentro do ServerHandler, acoplando protocolo e efeitos locais.
