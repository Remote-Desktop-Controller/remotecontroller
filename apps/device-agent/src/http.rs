use anyhow::{Context, Result, ensure};
use reqwest::{Client, Method, StatusCode, redirect::Policy};
use runtime_protocol::{RequestEnvelope, ResponseEnvelope, ToolDescriptor};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

const MAX_BODY: usize = 8 * 1024 * 1024;

/// Status stays typed so the durable outbox never treats a permanent orphan as a network retry.
#[derive(Debug)]
pub struct ServiceHttpError {
    pub status: StatusCode,
}
impl std::fmt::Display for ServiceHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_revoked() {
            write!(
                f,
                "AGENT_REVOKED: autorização do dispositivo expirada ou revogada; pareie novamente"
            )
        } else {
            write!(f, "Serviço respondeu HTTP {}", self.status.as_u16())
        }
    }
}
impl std::error::Error for ServiceHttpError {}
impl ServiceHttpError {
    pub fn is_revoked(&self) -> bool {
        matches!(
            self.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        )
    }
    pub fn permanent_result_reason(&self) -> Option<&'static str> {
        match self.status {
            StatusCode::NOT_FOUND | StatusCode::GONE => Some("remote_request_expired"),
            StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS => None,
            status if status.is_client_error() => Some("remote_result_rejected"),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct ServiceClient {
    client: Client,
    origin: Url,
    token: Option<String>,
}

#[derive(Serialize)]
pub struct PairRequest<'a> {
    pub name: &'a str,
    pub workspace_id: uuid::Uuid,
    pub tools: &'a [ToolDescriptor],
}

#[derive(Deserialize)]
pub struct PairStart {
    pub pairing_id: uuid::Uuid,
    pub pairing_code: String,
    pub verification_uri: String,
    pub poll_token: String,
    pub expires_at: i64,
}

#[derive(Deserialize)]
pub struct PairStatus {
    pub status: String,
    pub device_id: Option<uuid::Uuid>,
    pub device_token: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct RemoteCommand {
    pub request_id: uuid::Uuid,
    pub envelope: RequestEnvelope,
    #[serde(default)]
    pub server_budget_ms: Option<u64>,
    #[serde(skip)]
    pub received_at: Option<std::time::Instant>,
}

#[derive(Deserialize)]
pub struct PollResponse {
    pub commands: Vec<RemoteCommand>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteResult {
    pub request_id: uuid::Uuid,
    pub envelope: ResponseEnvelope,
}

impl ServiceClient {
    pub fn new(server: &str, allow_http_loopback: bool, token: Option<String>) -> Result<Self> {
        let origin = crate::validate_server(server, allow_http_loopback)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(35))
            .user_agent(concat!(
                "remotecontroller-agent/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self {
            client,
            origin,
            token,
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let url = self.origin.join(path)?;
        ensure!(
            url.origin() == self.origin.origin(),
            "Destino HTTP mudou de origem"
        );
        let mut request = self.client.request(method, url);
        if path == "/api/agent/result" {
            request = request.timeout(Duration::from_secs(5));
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .context("Serviço remoto indisponível")?;
        let status = response.status();
        if !status.is_success() {
            return Err(ServiceHttpError { status }.into());
        }
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_BODY as u64),
            "Resposta excede limite"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_BODY,
                "Resposta excede limite"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn pair(&self, request: &PairRequest<'_>) -> Result<PairStart> {
        self.request(
            Method::POST,
            "/api/pairings",
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn pairing_status(&self, id: uuid::Uuid, poll_token: &str) -> Result<PairStatus> {
        self.request(
            Method::POST,
            &format!("/api/pairings/{id}/poll"),
            Some(serde_json::json!({"poll_token":poll_token})),
        )
        .await
    }
    pub async fn heartbeat(
        &self,
        workspace_id: uuid::Uuid,
        tools: &[ToolDescriptor],
    ) -> Result<()> {
        let _: serde_json::Value = self
            .request(
                Method::POST,
                "/api/agent/heartbeat",
                Some(serde_json::json!({"workspace_id":workspace_id,"tools":tools})),
            )
            .await?;
        Ok(())
    }
    pub async fn poll(&self) -> Result<PollResponse> {
        let started = std::time::Instant::now();
        let mut response: PollResponse = self.request(Method::GET, "/api/agent/poll", None).await?;
        for command in &mut response.commands {
            if let Some(budget) = command.server_budget_ms {
                command.server_budget_ms =
                    Some(crate::budget::remaining_budget(budget, started.elapsed()));
                command.received_at = Some(std::time::Instant::now());
            }
        }
        ensure!(response.commands.len() <= 64, "Fila remota excede limite");
        Ok(response)
    }
    pub async fn result(&self, response: &RemoteResult) -> Result<()> {
        let _: serde_json::Value = self
            .request(
                Method::POST,
                "/api/agent/result",
                Some(serde_json::to_value(response)?),
            )
            .await?;
        Ok(())
    }
}
