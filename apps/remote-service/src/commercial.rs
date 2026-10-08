//! Optional commercial identity and access boundary. No provider calls hold the store lock.
use crate::{ApiError, ApiResult, AppState, account, security, store};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use libsql::params;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone)]
pub struct CommercialConfig {
    portal: String,
    secret: String,
    client: reqwest::Client,
}
impl CommercialConfig {
    pub fn new(portal: String, secret: String, allow_http_loopback: bool) -> anyhow::Result<Self> {
        let url = url::Url::parse(&portal)?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "commercial portal must be an origin"
        );
        anyhow::ensure!(
            url.scheme() == "https" || (allow_http_loopback && loopback && url.scheme() == "http"),
            "commercial portal requires HTTPS"
        );
        anyhow::ensure!(
            secret.len() >= 32
                && secret.len() <= 4096
                && !secret.bytes().any(|b| b.is_ascii_control()),
            "invalid commercial secret"
        );
        Ok(Self {
            portal: url.as_str().trim_end_matches('/').to_owned(),
            secret,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                .build()?,
        })
    }
    async fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> ApiResult<T> {
        let response = self
            .client
            .post(format!("{}{path}", self.portal))
            .bearer_auth(&self.secret)
            .json(&body)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        // Stream a bounded reply; never retain or log provider errors, URLs, or secrets.
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > 16384 {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| unavailable())
    }
}
fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "commercial_unavailable",
        "Não foi possível validar o acesso comercial",
    )
}
pub(crate) async fn login(state: &AppState, next: &str) -> ApiResult<Response> {
    let config = state
        .config
        .commercial
        .as_ref()
        .ok_or_else(ApiError::unauthorized)?;
    let nonce = security::token();
    let next = if next.len() <= 8192
        && !next.bytes().any(|b| b.is_ascii_control())
        && next.starts_with("/oauth/authorize?")
    {
        next
    } else {
        "/"
    };
    {
        let connection = state.store.lock().await;
        connection
            .execute(
                "DELETE FROM commercial_nonces WHERE expires_at<=?1",
                params![security::now()],
            )
            .await?;
        let row = store::first(&connection, "SELECT COUNT(*) FROM commercial_nonces", ()).await?;
        if store::integer(&row, 0)? >= 4096 {
            return Err(ApiError::quota());
        }
        connection
            .execute(
                "INSERT INTO commercial_nonces VALUES(?1,?2,?3)",
                params![security::hash(&nonce), next, security::now() + 300],
            )
            .await?;
    }
    let mut destination =
        url::Url::parse(&format!("{}/connect", config.portal)).map_err(|_| unavailable())?;
    destination
        .query_pairs_mut()
        .append_pair("state", &nonce)
        .append_pair(
            "redirect_uri",
            &format!("{}/auth/commercial/callback", state.config.public_url),
        );
    let mut response = Redirect::to(destination.as_str()).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        account::cookie_header(state, "rdc_commercial_nonce", &nonce, 300)
            .parse()
            .unwrap(),
    );
    Ok(response)
}
#[derive(Deserialize)]
pub(crate) struct Callback {
    ticket: String,
    state: String,
}
#[derive(Deserialize)]
struct Identity {
    subject: String,
    email: String,
    email_verified: bool,
    audience: String,
}
pub(crate) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> ApiResult<Response> {
    let config = state
        .config
        .commercial
        .as_ref()
        .ok_or_else(ApiError::unauthorized)?;
    if input.state.len() > 256
        || input.ticket.len() > 256
        || input.ticket.is_empty()
        || !account::cookie(&headers, "rdc_commercial_nonce")
            .is_some_and(|nonce| security::constant_eq(nonce, &input.state))
    {
        return Err(ApiError::unauthorized());
    }
    // Consume locally before exchange; failures require a fresh login and cannot race ticket replay.
    let next = {
        let connection = state.store.lock().await;
        let tx = connection
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        let mut rows = tx
            .query(
                "SELECT next FROM commercial_nonces WHERE nonce_hash=?1 AND expires_at>?2",
                params![security::hash(&input.state), security::now()],
            )
            .await?;
        let row = rows.next().await?.ok_or_else(ApiError::unauthorized)?;
        let next = store::text(&row, 0)?;
        drop(rows);
        tx.execute(
            "DELETE FROM commercial_nonces WHERE nonce_hash=?1",
            params![security::hash(&input.state)],
        )
        .await?;
        tx.commit().await?;
        next
    };
    let identity:Identity=config.post("/api/mcp/exchange",json!({"ticket":input.ticket,"state":input.state,"redirect_uri":format!("{}/auth/commercial/callback",state.config.public_url)})).await?;
    let subject = uuid::Uuid::parse_str(&identity.subject).map_err(|_| ApiError::forbidden())?;
    let email = identity.email.trim().to_lowercase();
    if subject.is_nil()
        || subject.to_string() != identity.subject
        || !identity.email_verified
        || identity.audience != state.config.public_url
        || email.len() > 254
        || !email.contains('@')
        || email
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(ApiError::forbidden());
    }
    let raw = security::token();
    {
        let connection = state.store.lock().await;
        let tx = connection
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        let mut rows = tx
            .query(
                "SELECT user_id FROM commercial_accounts WHERE subject=?1",
                params![identity.subject.clone()],
            )
            .await?;
        let existing = rows.next().await?.map(|r| store::text(&r, 0)).transpose()?;
        drop(rows);
        let id = match existing {
            Some(id) => id,
            None => {
                let mut rows = tx
                    .query(
                        "SELECT id FROM users WHERE email=?1",
                        params![email.clone()],
                    )
                    .await?;
                if rows.next().await?.is_some() {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "identity_migration_required",
                        "Conta local existente exige migração administrativa",
                    ));
                }
                drop(rows);
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO users VALUES(?1,?2,?3,?4)",
                    params![id.clone(), email, "commercial-only", security::now()],
                )
                .await?;
                tx.execute(
                    "INSERT INTO commercial_accounts VALUES(?1,?2)",
                    params![id.clone(), identity.subject],
                )
                .await?;
                id
            }
        };
        tx.execute(
            "DELETE FROM sessions WHERE expires_at<=?1",
            params![security::now()],
        )
        .await?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1 AND token_hash IN(SELECT token_hash FROM sessions WHERE user_id=?1 ORDER BY expires_at DESC LIMIT -1 OFFSET 9)",params![id.clone()]).await?;
        tx.execute(
            "INSERT INTO sessions VALUES(?1,?2,?3)",
            params![security::hash(&raw), id, security::now() + 86400],
        )
        .await?;
        tx.commit().await?;
    }
    let mut response = Redirect::to(&next).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        account::cookie_header(&state, "rdc_session", &raw, 86400)
            .parse()
            .unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        account::cookie_header(&state, "rdc_commercial_nonce", "", 0)
            .parse()
            .unwrap(),
    );
    Ok(response)
}
#[derive(Deserialize)]
struct Authorization {
    subject: String,
    operation_id: String,
    allowed: bool,
    state: String,
    expires_at: Option<i64>,
}
// Return the checked expiry so local transactions can reject a reply that expired in transit.
pub(crate) async fn authorize(
    state: &AppState,
    user: &str,
    operation: uuid::Uuid,
    execute: bool,
    setup: bool,
) -> ApiResult<Option<i64>> {
    let Some(config) = &state.config.commercial else {
        return Ok(None);
    };
    let subject = {
        let connection = state.store.lock().await;
        let row = store::first(
            &connection,
            "SELECT subject FROM commercial_accounts WHERE user_id=?1",
            params![user],
        )
        .await
        .map_err(|_| ApiError::forbidden())?;
        store::text(&row, 0)?
    };
    let reply:Authorization=config.post("/api/mcp/authorize",json!({"subject":subject,"operation_id":operation.to_string(),"mode":if execute{"execute"}else{"inspect"}})).await?;
    if reply.subject != subject || reply.operation_id != operation.to_string() {
        return Err(ApiError::forbidden());
    }
    if reply.allowed
        && reply.state == "ACTIVE"
        && reply
            .expires_at
            .is_some_and(|expiry| expiry > security::now())
    {
        return Ok(reply.expires_at);
    }
    if setup
        && !execute
        && !reply.allowed
        && reply.state == "READY_TO_ACTIVATE"
        && reply.expires_at.is_none()
    {
        return Ok(None);
    }
    Err(ApiError::forbidden())
}
