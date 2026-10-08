mod account;
mod commercial;
pub use commercial::CommercialConfig;
mod oauth;
mod pairing;
mod relay;
pub mod security;
mod store;
mod ui;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, path::Path, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

pub type ApiResult<T> = Result<T, ApiError>;
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
    pub challenge: Option<String>,
}
impl ApiError {
    pub fn bad(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "access_denied",
            "Acesso não autorizado",
        )
    }
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "Autenticação necessária",
        )
    }
    pub fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Registro não encontrado",
        )
    }
    pub fn quota() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
            "Limite atingido; tente novamente mais tarde",
        )
    }
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            challenge: None,
        }
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}
impl From<libsql::Error> for ApiError {
    fn from(error: libsql::Error) -> Self {
        tracing::error!(%error,"database operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "Falha interna de armazenamento",
        )
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        Self::bad("JSON inválido")
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(json!({"error":self.code,"message":self.message})),
        )
            .into_response();
        if let Some(challenge) = self.challenge
            && let Ok(value) = challenge.parse()
        {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }
}

#[derive(Clone)]
pub struct Config {
    pub commercial: Option<CommercialConfig>,
    pub public_url: String,
    pub allow_http_loopback: bool,
    pub redirect_uris: Vec<String>,
    pub poll_seconds: u64,
    pub call_seconds: u64,
    pub queue_limit: i64,
    pub result_limit: i64,
}
impl Config {
    pub fn new(
        public_url: String,
        allow_http_loopback: bool,
        redirect_uris: Vec<String>,
    ) -> anyhow::Result<Self> {
        let url = url::Url::parse(public_url.trim_end_matches('/'))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.query().is_none()
                && url.path() == "/",
            "public URL must be an origin without credentials/query/fragment"
        );
        anyhow::ensure!(
            url.scheme() == "https" || (allow_http_loopback && loopback && url.scheme() == "http"),
            "HTTPS required (explicit loopback exception only)"
        );
        let allow_http_loopback = allow_http_loopback && url.scheme() == "http";
        for redirect in &redirect_uris {
            anyhow::ensure!(
                security::redirect_allowed(
                    redirect,
                    std::slice::from_ref(redirect),
                    allow_http_loopback
                ),
                "invalid redirect URI"
            );
        }
        Ok(Self {
            commercial: None,
            public_url: url.as_str().trim_end_matches('/').to_owned(),
            allow_http_loopback,
            redirect_uris,
            poll_seconds: 25,
            call_seconds: 35,
            queue_limit: 64,
            result_limit: 256,
        })
    }
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.public_url)
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub store: Arc<store::Store>,
    pub config: Config,
    rates: Arc<Mutex<HashMap<String, (i64, u32)>>>,
    pub calls: Arc<Semaphore>,
    pub passwords: Arc<Semaphore>,
}
impl AppState {
    pub async fn rate(&self, key: String, limit: u32) -> ApiResult<()> {
        let now = security::now();
        let mut rates = self.rates.lock().await;
        rates.retain(|_, v| v.0 + 60 > now);
        if !rates.contains_key(&key) && rates.len() >= 4096 {
            return Err(ApiError::quota());
        }
        let entry = rates.entry(key).or_insert((now, 0));
        entry.1 += 1;
        if entry.1 > limit {
            Err(ApiError::quota())
        } else {
            Ok(())
        }
    }
}
pub async fn application(path: &Path, config: Config) -> ApiResult<Router> {
    let state = AppState {
        store: store::Store::open(path).await?,
        config,
        rates: Arc::new(Mutex::new(HashMap::new())),
        calls: Arc::new(Semaphore::new(128)),
        passwords: Arc::new(Semaphore::new(4)),
    };
    Ok(Router::new()
        .route("/", get(ui::home))
        .route("/login", get(ui::login).post(account::login_form))
        .route("/register", get(ui::register).post(account::register_form))
        .route("/auth/commercial/callback", get(commercial::callback))
        .route("/logout", post(account::logout_form))
        .route("/pair", get(ui::home).post(pairing::approve_form))
        .route("/devices/{id}/revoke", post(pairing::revoke_form))
        .route("/connections/{id}/revoke", post(oauth::revoke_form))
        .route("/api/auth/register", post(account::register_json))
        .route("/api/auth/login", post(account::login_json))
        .route("/api/auth/logout", post(account::logout_json))
        .route("/api/session", get(account::session_json))
        .route("/api/me", get(account::session_json))
        .route("/api/devices", get(pairing::list))
        .route("/api/devices/{id}/revoke", post(pairing::revoke_json))
        .route("/api/pairings", post(pairing::create))
        .route("/api/pairings/{id}/poll", post(pairing::poll))
        .route("/api/pairings/approve", post(pairing::approve_json))
        .route("/api/agent/heartbeat", post(relay::heartbeat))
        .route("/api/agent/poll", get(relay::poll))
        .route("/api/agent/result", post(relay::result))
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::metadata),
        )
        .route("/oauth/register", post(oauth::register))
        .route(
            "/oauth/authorize",
            get(oauth::authorize).post(oauth::consent),
        )
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .route(
            "/mcp",
            post(relay::mcp)
                .get(relay::unsupported_stream)
                .delete(relay::unsupported_stream),
        )
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state))
}
async fn boundary(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::ORIGIN)
        .is_some_and(|v| v.to_str().ok() != Some(state.config.public_url.as_str()))
    {
        return ApiError::forbidden().into_response();
    }
    let path = request.uri().path();
    if (request.method() == axum::http::Method::POST
        && (path.starts_with("/api/auth/")
            || path == "/login"
            || path == "/register"
            || path == "/api/pairings"
            || path == "/oauth/register"
            || path == "/oauth/token"))
        || (state.config.commercial.is_some()
            && matches!(path, "/login" | "/register" | "/auth/commercial/callback"))
    {
        // Forwarded headers are intentionally ignored; the front proxy should also rate limit.
        let ip = request
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|v| v.0.ip().to_string())
            .unwrap_or_else(|| "direct".into());
        if let Err(error) = state.rate(format!("ip:{ip}:{path}"), 30).await {
            return error.into_response();
        }
    }
    let mut response = next.run(request).await;
    // Keep extractor, size-limit and unsupported-route failures inside the JSON HTTP contract.
    if response.status().is_client_error()
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json") || v.starts_with("text/html"))
    {
        let (mut parts, _) = response.into_parts();
        parts.headers.remove(header::CONTENT_LENGTH);
        parts
            .headers
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        response=Response::from_parts(parts,axum::body::Body::from(json!({"error":"invalid_request","message":"Requisição inválida ou método não suportado"}).to_string()));
    }
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    // no-referrer makes browser POST navigations send Origin:null. Keep origin-only privacy while preserving same-origin form verification.
    headers.insert("referrer-policy", "strict-origin".parse().unwrap());
    headers.entry("content-security-policy").or_insert("default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
    response
}
pub(crate) fn bearer(headers: &HeaderMap) -> ApiResult<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|v| v.len() <= 256)
        .ok_or_else(ApiError::unauthorized)
}
