use crate::{ApiError, ApiResult, AppState, account, security, store, ui};
use axum::{
    Form, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use libsql::params;
use runtime_protocol::ToolDescriptor;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub(crate) struct PairInput {
    pub name: String,
    pub workspace_id: uuid::Uuid,
    pub tools: Vec<ToolDescriptor>,
}
pub(crate) fn validate_catalog(tools: &[ToolDescriptor]) -> ApiResult<String> {
    if tools.len() > 128 {
        return Err(ApiError::bad("Catálogo excede 128 ferramentas"));
    }
    let mut names = std::collections::HashSet::new();
    for tool in tools {
        if tool.name.is_empty()
            || tool.name.len() > 128
            || tool.name.starts_with("device.")
            || !tool
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !names.insert(&tool.name)
            || !tool.input_schema.is_object()
            || tool.description.len() > 8192
        {
            return Err(ApiError::bad("Catálogo de ferramentas inválido"));
        }
    }
    let encoded = serde_json::to_string(tools)?;
    if encoded.len() > 256 * 1024 {
        return Err(ApiError::bad("Catálogo muito grande"));
    }
    Ok(encoded)
}
pub(crate) async fn create(
    State(state): State<AppState>,
    Json(input): Json<PairInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err(ApiError::bad("Nome do computador inválido"));
    }
    let tools = validate_catalog(&input.tools)?;
    let id = uuid::Uuid::new_v4().to_string();
    let poll = security::token();
    // 40 random bits encoded as 8 Crockford characters; display grouped for human entry.
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let code: String = bytes
        .iter()
        .map(|v| alphabet[(v & 31) as usize] as char)
        .collect();
    let expires = security::now() + 600;
    let connection = state.store.lock().await;
    connection
        .execute(
            "DELETE FROM pairings WHERE expires_at<?1",
            params![security::now() - 3600],
        )
        .await?;
    let row = store::first(
        &connection,
        "SELECT COUNT(*) FROM pairings WHERE expires_at>?1",
        params![security::now()],
    )
    .await?;
    if store::integer(&row, 0)? >= 1024 {
        return Err(ApiError::quota());
    }
    connection.execute("INSERT INTO pairings(id,code_hash,poll_hash,name,workspace_id,tools,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id.clone(),security::hash(&code),security::hash(&poll),name.to_owned(),input.workspace_id.to_string(),tools,expires]).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"pairing_id":id,"pairing_code":format!("{}-{}",&code[..4],&code[4..]),"verification_uri":format!("{}/pair",state.config.public_url),"poll_token":poll,"expires_at":expires}),
        ),
    ))
}
#[derive(Deserialize)]
pub(crate) struct PollInput {
    poll_token: String,
}
pub(crate) async fn poll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<PollInput>,
) -> ApiResult<Json<Value>> {
    state
        .rate(
            format!("pair-poll:{}", security::hash(&input.poll_token)),
            30,
        )
        .await?;
    let connection = state.store.lock().await;
    let tx = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    let mut rows=tx.query("SELECT poll_hash,expires_at,user_id,device_id,delivered,name,workspace_id,tools FROM pairings WHERE id=?1",params![id.clone()]).await?;
    let row = rows.next().await?.ok_or_else(ApiError::unauthorized)?;
    drop(rows);
    if !security::constant_eq(&store::text(&row, 0)?, &security::hash(&input.poll_token)) {
        return Err(ApiError::unauthorized());
    }
    if store::integer(&row, 1)? <= security::now() {
        return Ok(Json(json!({"status":"expired"})));
    }
    let owner: Option<String> = row.get(2)?;
    let Some(owner) = owner else {
        return Ok(Json(json!({"status":"pending"})));
    };
    let device: Option<String> = row.get(3)?;
    if store::integer(&row, 4)? != 0 {
        return Ok(Json(json!({"status":"approved","device_id":device})));
    }
    let mut count=tx.query("SELECT COUNT(*),COALESCE(SUM(CASE WHEN revoked=0 THEN 1 ELSE 0 END),0) FROM devices WHERE user_id=?1",params![owner.clone()]).await?;
    let device_count = count.next().await?.ok_or_else(ApiError::not_found)?;
    drop(count);
    if store::integer(&device_count, 0)? >= 100 || store::integer(&device_count, 1)? >= 20 {
        return Err(ApiError::quota());
    }
    let device = uuid::Uuid::new_v4().to_string();
    let raw = security::token();
    tx.execute("INSERT INTO devices(id,user_id,token_hash,name,workspace_id,tools,heartbeat_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![device.clone(),owner,security::hash(&raw),store::text(&row,5)?,store::text(&row,6)?,store::text(&row,7)?,security::now()]).await?;
    tx.execute(
        "UPDATE pairings SET delivered=1,device_id=?2 WHERE id=?1",
        params![id, device.clone()],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"status":"approved","device_id":device,"device_token":raw}),
    ))
}
#[derive(Deserialize)]
pub(crate) struct ApproveInput {
    code: String,
    #[serde(default)]
    csrf: String,
}
async fn approve(state: &AppState, session: &account::WebSession, code: &str) -> ApiResult<()> {
    state
        .rate(format!("pair-approve:{}", session.user_id), 10)
        .await?;
    let code = code.replace(['-', ' '], "").to_ascii_uppercase();
    let connection = state.store.lock().await;
    let tx = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    let mut rows = tx
        .query(
            "SELECT expires_at,user_id FROM pairings WHERE code_hash=?1",
            params![security::hash(&code)],
        )
        .await?;
    let row = rows.next().await?.ok_or_else(ApiError::not_found)?;
    drop(rows);
    let owner: Option<String> = row.get(1)?;
    if let Some(ref existing_owner) = owner {
        if existing_owner == &session.user_id {
            // Already claimed and approved by this exact user!
            // Tolerates double-clicks, duplicate submits, and page refreshes.
            tx.commit().await?;
            return Ok(());
        }
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "already_used",
            "Código já utilizado por outra conta",
        ));
    }
    if store::integer(&row, 0)? <= security::now() {
        return Err(ApiError::bad("Código de pareamento expirado"));
    }
    let mut count = tx
        .query(
            "SELECT (SELECT COUNT(*) FROM devices WHERE user_id=?1 AND revoked=0)+(SELECT COUNT(*) FROM pairings WHERE user_id=?1 AND delivered=0 AND expires_at>?2)",
            params![session.user_id.clone(),security::now()],
        )
        .await?;
    if store::integer(&count.next().await?.ok_or_else(ApiError::not_found)?, 0)? >= 20 {
        return Err(ApiError::quota());
    }
    drop(count);
    tx.execute(
        "UPDATE pairings SET user_id=?1 WHERE code_hash=?2 AND user_id IS NULL",
        params![session.user_id.clone(), security::hash(&code)],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub(crate) async fn approve_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ApproveInput>,
) -> ApiResult<Json<Value>> {
    let session = account::session(&state, &headers).await?;
    account::check_csrf(&session, account::header_csrf(&headers))?;
    approve(&state, &session, &input.code).await?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn approve_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<ApproveInput>, axum::extract::rejection::FormRejection>,
) -> Response {
    let input = match form {
        Ok(Form(input)) => input,
        Err(_) => {
            return ui::form_error(
                ApiError::bad("Preencha o código de pareamento para continuar"),
                "/pair",
            );
        }
    };
    let session = match account::session(&state, &headers).await {
        Ok(session) => session,
        Err(err) => return ui::form_error(err, "/login"),
    };
    if let Err(err) = account::check_csrf(&session, Some(&input.csrf)) {
        return ui::form_error(err, "/pair");
    }
    match approve(&state, &session, &input.code).await {
        Ok(()) => Redirect::to("/?paired=1").into_response(),
        Err(err) => ui::form_error(err, "/pair"),
    }
}
pub(crate) async fn devices(state: &AppState, owner: &str) -> ApiResult<Vec<Value>> {
    let connection = state.store.lock().await;
    let mut rows=connection.query("SELECT id,name,workspace_id,heartbeat_at,revoked FROM devices WHERE user_id=?1 ORDER BY name LIMIT 100",params![owner]).await?;
    let mut devices = Vec::new();
    while let Some(row) = rows.next().await? {
        let heartbeat = store::integer(&row, 3)?;
        let revoked = store::integer(&row, 4)? != 0;
        devices.push(json!({"device_id":store::text(&row,0)?,"name":store::text(&row,1)?,"workspace_id":store::text(&row,2)?,"last_seen":heartbeat,"online":!revoked && heartbeat+90>security::now(),"revoked":revoked}));
    }
    Ok(devices)
}
pub(crate) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let session = account::session(&state, &headers).await?;
    Ok(Json(
        json!({"devices":devices(&state,&session.user_id).await?}),
    ))
}
async fn revoke(state: &AppState, owner: &str, id: &str) -> ApiResult<()> {
    let connection = state.store.lock().await;
    let tx = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    if tx
        .execute(
            "UPDATE devices SET revoked=1 WHERE id=?1 AND user_id=?2",
            params![id, owner],
        )
        .await?
        == 0
    {
        return Err(ApiError::not_found());
    }
    tx.execute(
        "DELETE FROM relay WHERE device_id=?1 AND result IS NULL",
        params![id],
    )
    .await?;
    tx.execute(
        "UPDATE grants SET selected_device=NULL WHERE user_id=?1 AND selected_device=?2",
        params![owner, id],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub(crate) async fn revoke_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let session = account::session(&state, &headers).await?;
    account::check_csrf(&session, account::header_csrf(&headers))?;
    revoke(&state, &session.user_id, &id).await?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn revoke_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(input): Form<account::CsrfForm>,
) -> ApiResult<Redirect> {
    let session = account::session(&state, &headers).await?;
    account::check_csrf(&session, Some(&input.csrf))?;
    revoke(&state, &session.user_id, &id).await?;
    Ok(Redirect::to("/"))
}
