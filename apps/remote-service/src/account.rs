use crate::{ApiError, ApiResult, AppState, security, store, ui};
use axum::{
    Form, Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use libsql::params;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct WebSession {
    pub user_id: String,
    pub email: String,
    pub csrf: String,
}
pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|v| v.trim().strip_prefix(&format!("{name}=")))
}
pub(crate) async fn session(state: &AppState, headers: &HeaderMap) -> ApiResult<WebSession> {
    let raw = cookie(headers, "rdc_session")
        .filter(|v| v.len() <= 256)
        .ok_or_else(ApiError::unauthorized)?;
    let connection = state.store.lock().await;
    let row=store::first(&connection,"SELECT s.user_id,u.email FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.expires_at>?2",params![security::hash(raw),security::now()]).await.map_err(|_|ApiError::unauthorized())?;
    Ok(WebSession {
        user_id: store::text(&row, 0)?,
        email: store::text(&row, 1)?,
        csrf: security::csrf(raw),
    })
}
pub(crate) fn check_csrf(session: &WebSession, provided: Option<&str>) -> ApiResult<()> {
    if provided.is_some_and(|v| security::constant_eq(v, &session.csrf)) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
pub(crate) fn header_csrf(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-csrf-token").and_then(|v| v.to_str().ok())
}
#[derive(Deserialize)]
pub(crate) struct Credentials {
    email: String,
    password: String,
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    next: String,
}
pub(crate) fn cookie_header(state: &AppState, name: &str, value: &str, max_age: i64) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if state.config.allow_http_loopback {
            ""
        } else {
            "; Secure"
        }
    )
}
fn credentials_valid(input: &Credentials) -> ApiResult<String> {
    let email = input.email.trim().to_lowercase();
    if email.len() > 254
        || email.len() < 3
        || !email.contains('@')
        || email
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
        || !(12..=256).contains(&input.password.len())
    {
        return Err(ApiError::bad(
            "Informe e-mail válido e senha com 12 a 256 caracteres",
        ));
    }
    Ok(email)
}
async fn authenticate(
    state: &AppState,
    input: Credentials,
    register: bool,
) -> ApiResult<(String, String)> {
    if state.config.commercial.is_some() {
        return Err(ApiError::forbidden());
    }
    let email = credentials_valid(&input)?;
    state.rate(format!("auth:{email}"), 8).await?;
    let permit = state
        .passwords
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::quota())?;
    let user_id = if register {
        let password = input.password;
        let encoded = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            security::password_hash(&password)
        })
        .await
        .map_err(|_| ApiError::bad("Falha ao proteger senha"))?
        .map_err(|_| ApiError::bad("Falha ao proteger senha"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let connection = state.store.lock().await;
        let added=connection.execute("INSERT OR IGNORE INTO users(id,email,password_hash,created_at) VALUES(?1,?2,?3,?4)",params![id.clone(),email,encoded,security::now()]).await?;
        if added == 0 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "account_exists",
                "Não foi possível cadastrar essa conta",
            ));
        }
        id
    } else {
        let row = {
            let connection = state.store.lock().await;
            store::first(
                &connection,
                "SELECT id,password_hash FROM users WHERE email=?1",
                params![email],
            )
            .await
            .ok()
        };
        // A valid Argon2 hash is computed for missing accounts as well, preventing a cheap timing oracle.
        let (id, encoded) = match row {
            Some(row) => (Some(store::text(&row, 0)?), Some(store::text(&row, 1)?)),
            None => (None, None),
        };
        let password = input.password;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match encoded {
                Some(encoded) => security::password_matches(&password, &encoded),
                None => {
                    let _ = security::password_hash(&password);
                    false
                }
            }
        })
        .await
        .map_err(|_| ApiError::unauthorized())?;
        if !valid {
            return Err(ApiError::unauthorized());
        }
        id.ok_or_else(ApiError::unauthorized)?
    };
    let raw = security::token();
    let connection = state.store.lock().await;
    connection
        .execute(
            "DELETE FROM sessions WHERE expires_at<=?1",
            params![security::now()],
        )
        .await?;
    // Bound sessions per account. Login creates a new session and evicts the oldest.
    connection.execute("DELETE FROM sessions WHERE user_id=?1 AND token_hash IN (SELECT token_hash FROM sessions WHERE user_id=?1 ORDER BY expires_at DESC LIMIT -1 OFFSET 9)",params![user_id.clone()]).await?;
    connection
        .execute(
            "INSERT INTO sessions VALUES(?1,?2,?3)",
            params![
                security::hash(&raw),
                user_id.clone(),
                security::now() + 86400
            ],
        )
        .await?;
    Ok((user_id, raw))
}
async fn auth_json(state: AppState, input: Credentials, register: bool) -> ApiResult<Response> {
    let (id, raw) = authenticate(&state, input, register).await?;
    let mut response = (
        if register {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({"user_id":id,"csrf_token":security::csrf(&raw)})),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&state, "rdc_session", &raw, 86400)
            .parse()
            .unwrap(),
    );
    Ok(response)
}
pub(crate) async fn register_json(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> ApiResult<Response> {
    auth_json(state, input, true).await
}
pub(crate) async fn login_json(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> ApiResult<Response> {
    auth_json(state, input, false).await
}
async fn auth_form(
    state: AppState,
    headers: HeaderMap,
    input: Credentials,
    register: bool,
) -> ApiResult<Response> {
    if !cookie(&headers, "rdc_login_csrf").is_some_and(|v| security::constant_eq(v, &input.csrf)) {
        return Err(ApiError::forbidden());
    }
    let next = if input.next.starts_with("/oauth/authorize?") {
        input.next.clone()
    } else {
        "/".into()
    };
    let (_, raw) = authenticate(&state, input, register).await?;
    let mut response = Redirect::to(&next).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&state, "rdc_session", &raw, 86400)
            .parse()
            .unwrap(),
    );
    Ok(response)
}
pub(crate) async fn register_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<Credentials>, axum::extract::rejection::FormRejection>,
) -> Response {
    account_form(state, headers, form, true).await
}
pub(crate) async fn login_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<Credentials>, axum::extract::rejection::FormRejection>,
) -> Response {
    account_form(state, headers, form, false).await
}
async fn account_form(
    state: AppState,
    headers: HeaderMap,
    form: Result<Form<Credentials>, axum::extract::rejection::FormRejection>,
    register: bool,
) -> Response {
    let path = if register { "/register" } else { "/login" };
    let input = match form {
        Ok(Form(input)) => input,
        Err(_) => {
            return ui::form_error(
                ApiError::bad("Preencha seu e-mail e sua senha para continuar"),
                path,
            );
        }
    };
    let suffix = if input.next.starts_with("/oauth/authorize?") {
        format!(
            "?{}",
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("next", &input.next)
                .finish()
        )
    } else {
        String::new()
    };
    match auth_form(state, headers, input, register).await {
        Ok(response) => response,
        Err(error) => ui::form_error(error, &format!("{path}{suffix}")),
    }
}
pub(crate) async fn session_json(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let session = session(&state, &headers).await?;
    Ok(Json(
        json!({"user_id":session.user_id,"email":session.email,"csrf_token":session.csrf}),
    ))
}
#[derive(Deserialize)]
pub(crate) struct CsrfForm {
    pub csrf: String,
}
async fn logout(state: &AppState, headers: &HeaderMap, csrf: Option<&str>) -> ApiResult<()> {
    let session = session(state, headers).await?;
    check_csrf(&session, csrf)?;
    let connection = state.store.lock().await;
    connection
        .execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            params![security::hash(
                cookie(headers, "rdc_session").unwrap_or_default()
            )],
        )
        .await?;
    Ok(())
}
pub(crate) async fn logout_json(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    logout(&state, &headers, header_csrf(&headers)).await?;
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&state, "rdc_session", "", 0).parse().unwrap(),
    );
    Ok(response)
}
pub(crate) async fn logout_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> ApiResult<Response> {
    logout(&state, &headers, Some(&form.csrf)).await?;
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&state, "rdc_session", "", 0).parse().unwrap(),
    );
    Ok(response)
}
