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
pub(crate) struct Usage {
    subject: String,
    operation_id: String,
    pub allowed: bool,
    state: String,
    pub plan: String,
    pub expires_at: Option<i64>,
    remaining_seconds: f64,
    lease_remaining_ms: u64,
    #[serde(skip)]
    elapsed: Duration,
    resets_at: i64,
    error_code: Option<String>,
}
impl Usage {
    pub fn remaining_ms(&self) -> u64 {
        self.lease_remaining_ms.min(120000).saturating_sub(
            self.elapsed
                .as_micros()
                .div_ceil(1000)
                .min(u64::MAX as u128) as u64,
        )
    }
    pub fn check(&self) -> ApiResult<()> {
        if self.allowed && self.state == "ACTIVE" && self.remaining_ms() > 0 {
            return Ok(());
        }
        let mut error = ApiError::new(
            StatusCode::PAYMENT_REQUIRED,
            if self.error_code.as_deref() == Some("FREE_DAILY_LIMIT_REACHED") {
                "FREE_DAILY_LIMIT_REACHED"
            } else {
                "commercial_access_required"
            },
            "Limite de uso indisponível; aguarde a renovação ou faça upgrade para Premium",
        );
        error.details = Some(
            json!({"remaining_seconds":self.remaining_seconds,"resets_at":self.resets_at,"upgrade_url":"/dashboard"}),
        );
        Err(error)
    }
}
pub(crate) async fn usage(
    state: &AppState,
    user: &str,
    operation: uuid::Uuid,
    action: &str,
    max_seconds: u64,
) -> ApiResult<Option<Usage>> {
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
    let started = std::time::Instant::now();
    let mut reply:Usage=config.post("/api/mcp/usage",json!({"subject":subject,"operation_id":operation.to_string(),"action":action,"max_seconds":max_seconds.clamp(1,120)})).await?;
    reply.elapsed = started.elapsed();
    if reply.subject != subject
        || reply.operation_id != operation.to_string()
        || !matches!(reply.plan.as_str(), "FREE" | "PREMIUM" | "ADMIN")
        || !reply.remaining_seconds.is_finite()
        || reply.remaining_seconds < 0.0
    {
        return Err(ApiError::forbidden());
    }
    Ok(Some(reply))
}
// Discovery checks identity and portal availability, but does not consume quota.
pub(crate) async fn authorize(
    state: &AppState,
    user: &str,
    operation: uuid::Uuid,
    _execute: bool,
    setup: bool,
) -> ApiResult<Option<i64>> {
    let Some(reply) = usage(state, user, operation, "inspect", 120).await? else {
        return Ok(None);
    };
    if setup {
        return Ok(reply.expires_at);
    }
    reply.check()?;
    Ok(reply.expires_at)
}
pub(crate) fn bounded(reply: &Usage, tool: &str) -> ApiResult<()> {
    if reply.plan == "FREE" && matches!(tool, "terminal.open" | "terminal.write" | "process.spawn")
    {
        return Err(ApiError::new(
            StatusCode::PAYMENT_REQUIRED,
            "FREE_OPERATION_NOT_BOUNDED",
            "Operação persistente exige Premium até haver encerramento verificável",
        ));
    }
    Ok(())
}
