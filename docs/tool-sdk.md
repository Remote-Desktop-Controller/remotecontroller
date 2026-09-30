# SDK interno de tools

Implemente runtime_ports::Tool (Send + Sync). metadata retorna nome/version,
description, capabilities, input/output schema, risk e Workload. execute recebe
Value e ToolContext com workspace/operation/trace, cancellation, progress e
controle de operações. Retorna Result<Value,PortError>. Tool não importa rmcp.

```rust,ignore
#[async_trait::async_trait]
impl runtime_ports::Tool for MyTool {
    fn metadata(&self) -> runtime_ports::ToolMetadata { self.metadata.clone() }
    async fn execute(&self, input: serde_json::Value, ctx: runtime_ports::ToolContext)
        -> runtime_ports::Result<serde_json::Value> {
        if ctx.cancellation.is_cancelled() {
            return Err(runtime_ports::PortError::Cancelled);
        }
        self.adapter.execute(input, ctx).await
    }
}
```

Este exemplo ilustra o contrato; tools reais estão em tools.rs/analysis.rs.
Registrar Arc<dyn Tool> no Registry do daemon é suficiente; nomes duplicados
falham. Gateway solicita catálogo dinamicamente. Schema atual é um subconjunto
fechado validado recursivamente (type, properties, required, enum, limits, uuid);
não pressupor suporte a referências externas ou JSON Schema completo.

Não executar CPU pesada em workers Tokio. Cooperar com cancellation em loops e
aguardar cleanup antes de retornar. Deadline cancela o token e aguarda essa
cooperação; uma implementação de plugin que ignore o token não possui encerramento
forçado de future. Registrar capability adicional quando efeitos ultrapassarem
filesystem/processos do contrato. Não logar input/raw output indiscriminadamente.

Usar adapters confinados. Escritas devem participar do journal, não chamar
std::fs diretamente. Testar tool com tempdir e depois por IPC/MCP. Riscos e grants
são aplicados no daemon, não confiados a annotations do cliente MCP.
