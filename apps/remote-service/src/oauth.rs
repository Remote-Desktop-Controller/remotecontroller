use crate::{ApiError, ApiResult, AppState, account, security, store, ui};
use axum::{
    Form, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use libsql::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const SCOPES: &str = "devices:read tools:read tools:execute";
pub(crate) async fn metadata(State(state): State<AppState>) -> Json<Value> {
    let base = &state.config.public_url;
    Json(
        json!({"issuer":base,"authorization_endpoint":format!("{base}/oauth/authorize"),"token_endpoint":format!("{base}/oauth/token"),"registration_endpoint":format!("{base}/oauth/register"),"revocation_endpoint":format!("{base}/oauth/revoke"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":SCOPES.split(' ').collect::<Vec<_>>(),"client_id_metadata_document_supported":false}),
    )
}
pub(crate) async fn resource_metadata(State(state): State<AppState>) -> Json<Value> {
    Json(
        json!({"resource":state.config.resource(),"authorization_servers":[state.config.public_url],"scopes_supported":SCOPES.split(' ').collect::<Vec<_>>(),"bearer_methods_supported":["header"],"resource_name":"Remote Desktop Commander"}),
    )
}
#[derive(Deserialize)]
pub(crate) struct RegisterInput {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
}
pub(crate) async fn register(
    State(state): State<AppState>,
    Json(input): Json<RegisterInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if input.redirect_uris.is_empty()
        || input.redirect_uris.len() > 8
        || !input.redirect_uris.iter().all(|v| {
            security::redirect_allowed(
                v,
                &state.config.redirect_uris,
                state.config.allow_http_loopback,
            )
        })
        || input
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|v| v != "none")
        || input.grant_types.as_ref().is_some_and(|v| {
            v.iter()
                .any(|v| v != "authorization_code" && v != "refresh_token")
        })
        || input
            .response_types
            .as_ref()
            .is_some_and(|v| v.iter().any(|v| v != "code"))
    {
        return Err(ApiError::bad(
            "Cliente ou endereço de retorno não permitido",
        ));
    }
    let name = input.client_name.unwrap_or_else(|| "Cliente MCP".into());
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err(ApiError::bad("Nome do cliente inválido"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let connection = state.store.lock().await;
    let count = store::first(&connection, "SELECT COUNT(*) FROM clients", ()).await?;
    if store::integer(&count, 0)? >= 10000 {
        return Err(ApiError::quota());
    }
    connection
        .execute(
            "INSERT INTO clients VALUES(?1,?2,?3,?4)",
            params![
                id.clone(),
                name.clone(),
                serde_json::to_string(&input.redirect_uris)?,
                security::now()
            ],
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"client_id":id,"client_name":name,"redirect_uris":input.redirect_uris,"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"],"client_id_issued_at":security::now()}),
        ),
    ))
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Authorization {
    client_id: String,
    redirect_uri: String,
    response_type: String,
    code_challenge: String,
    code_challenge_method: String,
    resource: String,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default)]
    state: String,
}
fn default_scope() -> String {
    SCOPES.into()
}
fn valid_scope(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 128
        && value
            .split_whitespace()
            .all(|v| SCOPES.split(' ').any(|s| s == v))
}
async fn validate_authorization(state: &AppState, input: &Authorization) -> ApiResult<String> {
    if input.response_type != "code"
        || input.code_challenge_method != "S256"
        || input.resource != state.config.resource()
        || input.code_challenge.len() != 43
        || !input
            .code_challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        || !valid_scope(&input.scope)
        || input.state.len() > 1024
    {
        return Err(ApiError::bad(
            "Autorização exige PKCE S256, resource e escopos válidos",
        ));
    }
    let connection = state.store.lock().await;
    let row = store::first(
        &connection,
        "SELECT name,redirects FROM clients WHERE id=?1",
        params![input.client_id.clone()],
    )
    .await
    .map_err(|_| ApiError::bad("Cliente desconhecido"))?;
    let redirects: Vec<String> = serde_json::from_str(&store::text(&row, 1)?)?;
    if !redirects.contains(&input.redirect_uri) {
        return Err(ApiError::bad("Endereço de retorno não registrado"));
    }
    store::text(&row, 0)
}
pub(crate) async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Authorization>,
) -> ApiResult<Response> {
    let name = validate_authorization(&state, &input).await?;
    let session = match account::session(&state, &headers).await {
        Ok(session) => session,
        Err(_) => {
            let mut next = url::Url::parse(&format!("{}/oauth/authorize", state.config.public_url))
                .map_err(|_| ApiError::bad("URL inválida"))?;
            {
                let mut pairs = next.query_pairs_mut();
                for (k, v) in authorization_fields(&input) {
                    pairs.append_pair(k, &v);
                }
            }
            let local = format!("{}?{}", next.path(), next.query().unwrap_or_default());
            let mut login = url::Url::parse(&format!("{}/login", state.config.public_url))
                .map_err(|_| ApiError::bad("URL inválida"))?;
            login.query_pairs_mut().append_pair("next", &local);
            return Ok(
                Redirect::to(&format!("/login?{}", login.query().unwrap_or_default()))
                    .into_response(),
            );
        }
    };
    let mut fields = String::new();
    for (k, v) in authorization_fields(&input) {
        fields.push_str(&format!(
            "<input type=hidden name=\"{}\" value=\"{}\">",
            k,
            security::escape(&v)
        ));
    }
    let html = format!(
        "<h1>Autorizar conexão</h1><p><strong>{}</strong> solicita acesso aos seus computadores e ferramentas.</p><p>Conta: {}</p><p>Permissões: {}</p><p>Cada comando continua sujeito às permissões e aprovações locais do computador.</p><form method=post action=/oauth/authorize>{fields}<input type=hidden name=csrf value=\"{}\"><button name=decision value=allow>Autorizar</button> <button class=secondary name=decision value=deny>Cancelar</button></form>",
        security::escape(&name),
        security::escape(&session.email),
        security::escape(&ui::scope_description(&input.scope)),
        session.csrf
    );
    let callback_origin = url::Url::parse(&input.redirect_uri)
        .map_err(|_| ApiError::bad("Retorno inválido"))?
        .origin()
        .ascii_serialization();
    let mut response = Html(ui::page("Autorizar conexão", &html)).into_response();
    response.headers_mut().insert("content-security-policy",format!("default-src 'none'; style-src 'unsafe-inline'; form-action 'self' {callback_origin}; frame-ancestors 'none'; base-uri 'none'").parse().map_err(|_|ApiError::bad("Retorno inválido"))?);
    Ok(response)
}
fn authorization_fields(input: &Authorization) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", input.client_id.clone()),
        ("redirect_uri", input.redirect_uri.clone()),
        ("response_type", input.response_type.clone()),
        ("code_challenge", input.code_challenge.clone()),
        ("code_challenge_method", input.code_challenge_method.clone()),
        ("resource", input.resource.clone()),
        ("scope", input.scope.clone()),
        ("state", input.state.clone()),
    ]
}
#[derive(Deserialize)]
pub(crate) struct Consent {
    #[serde(flatten)]
    authorization: Authorization,
    csrf: String,
    decision: String,
}
pub(crate) async fn consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<Consent>,
) -> ApiResult<Redirect> {
    let session = account::session(&state, &headers).await?;
    account::check_csrf(&session, Some(&input.csrf))?;
    validate_authorization(&state, &input.authorization).await?;
    state
        .rate(format!("consent:{}", session.user_id), 20)
        .await?;
    let mut redirect = url::Url::parse(&input.authorization.redirect_uri)
        .map_err(|_| ApiError::bad("Retorno inválido"))?;
    if input.decision != "allow" {
        redirect
            .query_pairs_mut()
            .append_pair("error", "access_denied")
            .append_pair("state", &input.authorization.state);
        return Ok(Redirect::to(redirect.as_str()));
    }
    let code = security::token();
    let connection = state.store.lock().await;
    connection
        .execute(
            "DELETE FROM authorization_codes WHERE expires_at<?1",
            params![security::now() - 3600],
        )
        .await?;
    connection.execute("INSERT INTO authorization_codes(code_hash,user_id,client_id,redirect_uri,challenge,resource,scope,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![security::hash(&code),session.user_id,input.authorization.client_id,input.authorization.redirect_uri,input.authorization.code_challenge,input.authorization.resource,input.authorization.scope,security::now()+300]).await?;
    redirect
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &input.authorization.state);
    Ok(Redirect::to(redirect.as_str()))
}
#[derive(Deserialize)]
pub(crate) struct TokenInput {
    grant_type: String,
    client_id: String,
    resource: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    refresh_token: String,
}
fn invalid_grant() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_grant",
        "Código ou token inválido, expirado ou já utilizado",
    )
}
pub(crate) async fn token(
    State(state): State<AppState>,
    Form(input): Form<TokenInput>,
) -> ApiResult<Json<Value>> {
    if input.resource != state.config.resource() {
        return Err(invalid_grant());
    }
    let connection = state.store.lock().await;
    let tx = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    let (owner, scope, family, selected, refresh_expiry) = match input.grant_type.as_str() {
        "authorization_code" => {
            let mut rows=tx.query("SELECT user_id,client_id,redirect_uri,challenge,resource,scope,expires_at,consumed FROM authorization_codes WHERE code_hash=?1",params![security::hash(&input.code)]).await?;
            let row = rows.next().await?.ok_or_else(invalid_grant)?;
            drop(rows);
            if store::text(&row, 1)? != input.client_id
                || store::text(&row, 2)? != input.redirect_uri
                || store::text(&row, 4)? != input.resource
                || store::integer(&row, 6)? <= security::now()
                || store::integer(&row, 7)? != 0
                || !security::pkce_matches(&input.code_verifier, &store::text(&row, 3)?)
            {
                return Err(invalid_grant());
            }
            tx.execute(
                "UPDATE authorization_codes SET consumed=1 WHERE code_hash=?1 AND consumed=0",
                params![security::hash(&input.code)],
            )
            .await?;
            (
                store::text(&row, 0)?,
                store::text(&row, 5)?,
                uuid::Uuid::new_v4().to_string(),
                None,
                security::now() + 30 * 86400,
            )
        }
        "refresh_token" => {
            let mut rows=tx.query("SELECT user_id,client_id,resource,scope,family,refresh_expires_at,revoked,rotated,selected_device FROM grants WHERE refresh_hash=?1",params![security::hash(&input.refresh_token)]).await?;
            let row = rows.next().await?.ok_or_else(invalid_grant)?;
            drop(rows);
            let family = store::text(&row, 4)?;
            if store::text(&row, 1)? != input.client_id || store::text(&row, 2)? != input.resource {
                return Err(invalid_grant());
            }
            if store::integer(&row, 7)? != 0 {
                tx.execute(
                    "UPDATE grants SET revoked=1 WHERE family=?1",
                    params![family],
                )
                .await?;
                tx.commit().await?;
                return Err(invalid_grant());
            }
            if store::integer(&row, 6)? != 0 || store::integer(&row, 5)? <= security::now() {
                return Err(invalid_grant());
            }
            tx.execute(
                "UPDATE grants SET rotated=1,revoked=1 WHERE refresh_hash=?1",
                params![security::hash(&input.refresh_token)],
            )
            .await?;
            let selected: Option<String> = row.get(8)?;
            (
                store::text(&row, 0)?,
                store::text(&row, 3)?,
                family,
                selected,
                store::integer(&row, 5)?,
            )
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "Fluxo OAuth não suportado",
            ));
        }
    };
    let access = security::token();
    let refresh = security::token();
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "DELETE FROM grants WHERE refresh_expires_at<?1",
        params![security::now()],
    )
    .await?;
    let mut count = tx
        .query(
            "SELECT COUNT(*) FROM grants WHERE user_id=?1",
            params![owner.clone()],
        )
        .await?;
    if store::integer(&count.next().await?.ok_or_else(ApiError::not_found)?, 0)? >= 4096 {
        return Err(ApiError::quota());
    }
    drop(count);
    tx.execute("INSERT INTO grants(id,family,user_id,client_id,access_hash,refresh_hash,resource,scope,expires_at,refresh_expires_at,selected_device) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![id,family,owner,input.client_id,security::hash(&access),security::hash(&refresh),input.resource,scope.clone(),security::now()+3600,refresh_expiry,selected]).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"access_token":access,"token_type":"Bearer","expires_in":3600,"refresh_token":refresh,"scope":scope}),
    ))
}
#[derive(Deserialize)]
pub(crate) struct RevokeInput {
    token: String,
    client_id: String,
}
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Form(input): Form<RevokeInput>,
) -> ApiResult<Json<Value>> {
    let connection = state.store.lock().await;
    connection.execute("UPDATE grants SET revoked=1 WHERE family IN (SELECT family FROM grants WHERE client_id=?1 AND (access_hash=?2 OR refresh_hash=?2))",params![input.client_id,security::hash(&input.token)]).await?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn revoke_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(family): Path<String>,
    Form(input): Form<account::CsrfForm>,
) -> ApiResult<Redirect> {
    let session = account::session(&state, &headers).await?;
    account::check_csrf(&session, Some(&input.csrf))?;
    let connection = state.store.lock().await;
    connection
        .execute(
            "UPDATE grants SET revoked=1 WHERE family=?1 AND user_id=?2",
            params![family, session.user_id],
        )
        .await?;
    Ok(Redirect::to("/"))
}
pub(crate) struct Grant {
    pub id: String,
    pub family: String,
    pub user_id: String,
    pub scope: String,
    pub selected_device: Option<String>,
}
pub(crate) async fn grant(state: &AppState, headers: &HeaderMap) -> ApiResult<Grant> {
    let authenticate=async {
        let token=crate::bearer(headers)?;let connection=state.store.lock().await;
        let row=store::first(&connection,"SELECT id,family,user_id,scope,selected_device FROM grants WHERE access_hash=?1 AND expires_at>?2 AND revoked=0 AND resource=?3",params![security::hash(token),security::now(),state.config.resource()]).await.map_err(|_|ApiError::unauthorized())?;
        Ok(Grant{id:store::text(&row,0)?,family:store::text(&row,1)?,user_id:store::text(&row,2)?,scope:store::text(&row,3)?,selected_device:row.get(4)?})
    }.await;
    authenticate.map_err(|mut error:ApiError|{error.challenge=Some(format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"{}\"",state.config.public_url,SCOPES));error})
}
pub(crate) fn require_scope(state: &AppState, grant: &Grant, scope: &str) -> ApiResult<()> {
    if grant.scope.split_whitespace().any(|v| v == scope) {
        return Ok(());
    }
    let mut error = ApiError::new(
        StatusCode::FORBIDDEN,
        "insufficient_scope",
        "Permissão OAuth insuficiente",
    );
    error.challenge = Some(format!(
        "Bearer error=\"insufficient_scope\", scope=\"{scope}\", resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
        state.config.public_url
    ));
    Err(error)
}
