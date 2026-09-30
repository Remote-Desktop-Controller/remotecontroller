//! Protocol adapter only. No execution, workspace filesystem or storage dependency.
use anyhow::Result;
use clap::Parser;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext,
    transport::stdio,
};
use runtime_protocol::{ErrorCode, Metadata, Request, RequestEnvelope, ToolDescriptor};
use runtime_transport::Client;
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

#[derive(Parser)]
#[command(
    version,
    about = "MCP stdio gateway to a separately running local daemon"
)]
struct Args {
    #[arg(long)]
    endpoint: String,
    #[arg(long)]
    token_file: PathBuf,
}
#[derive(Clone)]
struct Gateway {
    client: Client,
    workspace: uuid::Uuid,
    tools: Arc<RwLock<Vec<Tool>>>,
    timeout_ms: u64,
}
impl Gateway {
    async fn connect(client: Client) -> Result<Self> {
        let request = RequestEnvelope {
            meta: Metadata::new(None, 10_000),
            payload: Request::Catalog,
        };
        let catalog = client
            .request(&request, |_| {})
            .await?
            .payload
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
        let workspace = catalog["workspace_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing workspace"))?
            .parse()?;
        let timeout_ms = catalog["operation_timeout_ms"].as_u64().unwrap_or(60_000);
        let tools = Self::decode_tools(&catalog)?;
        Ok(Self {
            client,
            workspace,
            tools: Arc::new(RwLock::new(tools)),
            timeout_ms,
        })
    }
    fn decode_tools(value: &serde_json::Value) -> Result<Vec<Tool>> {
        let descriptors: Vec<ToolDescriptor> = serde_json::from_value(value["tools"].clone())?;
        descriptors
            .into_iter()
            .map(|d| {
                let schema = d
                    .input_schema
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("invalid tool schema"))?
                    .clone();
                let mut tool = Tool::new(d.name, d.description, Arc::new(schema));
                tool.annotations = Some(
                    ToolAnnotations::new()
                        .read_only(d.risk == "read")
                        .destructive(d.risk != "read")
                        .open_world(d.risk == "high"),
                );
                Ok(tool)
            })
            .collect()
    }
}
impl ServerHandler for Gateway {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("local-runtime-gateway",env!("CARGO_PKG_VERSION")))
            .with_instructions("Tools execute in the separate local daemon, under its workspace policy. Outputs include operation IDs for status/cancellation and retry auditing.")
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools
            .read()
            .ok()?
            .iter()
            .find(|t| t.name == name)
            .cloned()
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        let request = RequestEnvelope {
            meta: Metadata::new(Some(self.workspace), 10_000),
            payload: Request::Catalog,
        };
        let response = self
            .client
            .request(&request, |_| {})
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let value = response
            .payload
            .map_err(|e| ErrorData::internal_error(e.message, None))?;
        let tools = Self::decode_tools(&value)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        *self
            .tools
            .write()
            .map_err(|_| ErrorData::internal_error("catalog lock", None))? = tools.clone();
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let mut meta = Metadata::new(Some(self.workspace), self.timeout_ms + 30_000);
        // Hosts can explicitly retain the same identity across independent MCP retries.
        if let Some(id) = context
            .meta
            .0
            .get("runtimeOperationId")
            .and_then(serde_json::Value::as_str)
        {
            meta.operation_id = id
                .parse()
                .map_err(|_| ErrorData::invalid_params("invalid runtimeOperationId", None))?;
        }
        let request = RequestEnvelope {
            meta: meta.clone(),
            payload: Request::Execute {
                tool: params.name.to_string(),
                input: serde_json::Value::Object(params.arguments.unwrap_or_default()),
            },
        };
        let cancel_client = self.client.clone();
        let workspace = self.workspace;
        let operation = meta.operation_id;
        let ct = context.ct.clone();
        let cancel_watch = tokio::spawn(async move {
            ct.cancelled().await;
            for _ in 0..20 {
                let request = RequestEnvelope {
                    meta: Metadata::new(Some(workspace), 5_000),
                    payload: Request::Cancel {
                        operation_id: operation,
                    },
                };
                match cancel_client.request(&request, |_| {}).await {
                    Ok(r) if r.payload.as_ref().is_ok_and(|_| true) => break,
                    Ok(r)
                        if r.payload
                            .as_ref()
                            .is_err_and(|e| e.code == ErrorCode::NotFound) =>
                    {
                        tokio::time::sleep(Duration::from_millis(50)).await
                    }
                    _ => break,
                }
            }
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel::<runtime_protocol::EventEnvelope>(32);
        let peer = context.peer.clone();
        let token = context.meta.get_progress_token();
        let forward = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Some(token) = token.clone() {
                    let mut notification =
                        ProgressNotificationParam::new(token, event.completed as f64);
                    notification.total = Some(event.total as f64);
                    notification.message = Some(event.message);
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        peer.notify_progress(notification),
                    )
                    .await;
                }
            }
        });
        let response = self
            .client
            .request(&request, |event| {
                let _ = tx.try_send(event);
            })
            .await;
        drop(tx);
        cancel_watch.abort();
        let _ = forward.await;
        Ok(match response {
            Ok(response) => match response.payload {
                Ok(value) if value["error"].is_null() => CallToolResult::structured(value),
                Ok(value) => CallToolResult::structured_error(value),
                Err(error) => CallToolResult::error(vec![ContentBlock::text(error.message)]),
            },
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
        .into())
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let secret = runtime_transport::read_secret(&args.token_file)?;
    let gateway = Gateway::connect(Client::new(args.endpoint, secret)).await?;
    tracing::info!("MCP gateway connected");
    gateway.serve(stdio()).await?.waiting().await?;
    Ok(())
}
