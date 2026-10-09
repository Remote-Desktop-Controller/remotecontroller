use crate::{ApiError, ApiResult, AppState, commercial, oauth, pairing, security, store};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use libsql::params;
use runtime_protocol::{Metadata, Request, RequestEnvelope, ResponseEnvelope, ToolDescriptor};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

struct Device {
    id: String,
    owner: String,
    workspace: uuid::Uuid,
    tools: Vec<ToolDescriptor>,
}
async fn agent(state: &AppState, headers: &HeaderMap) -> ApiResult<Device> {
    let raw = crate::bearer(headers)?;
    let connection = state.store.lock().await;
    let row = store::first(
        &connection,
        "SELECT id,user_id,workspace_id,tools FROM devices WHERE token_hash=?1 AND revoked=0",
        params![security::hash(raw)],
    )
    .await
    .map_err(|_| ApiError::unauthorized())?;
    decode_device(&row)
}
fn decode_device(row: &libsql::Row) -> ApiResult<Device> {
    Ok(Device {
        id: store::text(row, 0)?,
        owner: store::text(row, 1)?,
        workspace: store::text(row, 2)?
            .parse()
            .map_err(|_| ApiError::bad("Workspace inválido"))?,
        tools: serde_json::from_str(&store::text(row, 3)?)?,
    })
}
async fn owned_device(state: &AppState, owner: &str, id: &str) -> ApiResult<Device> {
    let connection = state.store.lock().await;
    let row=store::first(&connection,"SELECT id,user_id,workspace_id,tools FROM devices WHERE id=?1 AND user_id=?2 AND revoked=0",params![id,owner]).await?;
    decode_device(&row)
}
#[derive(Deserialize)]
pub(crate) struct Heartbeat {
    workspace_id: uuid::Uuid,
    tools: Vec<ToolDescriptor>,
}
pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Heartbeat>,
) -> ApiResult<Json<Value>> {
    let device = agent(&state, &headers).await?;
    if device.workspace != input.workspace_id {
        return Err(ApiError::forbidden());
    }
    let tools = pairing::validate_catalog(&input.tools)?;
    let connection = state.store.lock().await;
    if connection
        .execute(
            "UPDATE devices SET tools=?1,heartbeat_at=?2 WHERE id=?3 AND revoked=0",
            params![tools, security::now(), device.id],
        )
        .await?
        == 0
    {
        return Err(ApiError::unauthorized());
    }
    Ok(Json(json!({"status":"ok","heartbeat_interval_seconds":30})))
}
pub(crate) async fn poll(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _permit = state
        .calls
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::quota())?;
    let device = agent(&state, &headers).await?;
    state.rate(format!("agent-poll:{}", device.id), 120).await?;
    let start = Instant::now();
    loop {
        // Select candidates without a transaction across the authoritative HTTP reservation.
        let candidates = {
            let connection = state.store.lock().await;
            let mut rows=connection.query("SELECT r.id,r.envelope,c.deadline FROM relay r LEFT JOIN commercial_dispatches c ON c.request_id=r.id WHERE device_id=?1 AND user_id=?2 AND result IS NULL AND r.deadline>?3 AND lease_until<=?4 AND NOT EXISTS(SELECT 1 FROM commercial_dispatch_claims q WHERE q.request_id=r.id AND q.claim_until>?4) AND (?5=0 OR EXISTS(SELECT 1 FROM commercial_admissions a WHERE a.request_id=r.id AND a.authorized=1)) AND EXISTS(SELECT 1 FROM grants g WHERE g.family=r.grant_family AND g.revoked=0 AND g.expires_at>?4) ORDER BY CASE WHEN json_extract(envelope,'$.payload.Execute.tool') IN ('operation.status','operation.cancel','process.kill','terminal.close') THEN 0 ELSE 1 END,created_at LIMIT 8",params![device.id.clone(),device.owner.clone(),runtime_protocol::now_ms() as i64,security::now(),i64::from(state.config.commercial.is_some())]).await?;
            let mut values = Vec::new();
            while let Some(row) = rows.next().await? {
                values.push((
                    store::text(&row, 0)?,
                    serde_json::from_str::<RequestEnvelope>(&store::text(&row, 1)?)?,
                    row.get::<Option<i64>>(2)?,
                ));
            }
            values
        };
        let mut commands = Vec::new();
        let mut deferred_error = None;
        let mut budgets = std::collections::HashMap::new();
        for (id, mut envelope, persisted) in candidates {
            let recovery = recovery(&envelope.payload);
            let mut deadline = envelope.meta.deadline;
            let commercial = state.config.commercial.is_some() && !recovery;
            let claim = uuid::Uuid::new_v4().to_string();
            if commercial {
                let connection = state.store.lock().await;
                let changed=connection.execute("INSERT INTO commercial_dispatch_claims(request_id,token,claim_until) VALUES(?1,?2,?3) ON CONFLICT(request_id) DO UPDATE SET token=excluded.token,claim_until=excluded.claim_until WHERE commercial_dispatch_claims.claim_until<=?4",params![id.clone(),claim.clone(),security::now()+30,security::now()]).await?;
                if changed == 0 {
                    continue;
                }
            }
            let mut budget_end = None;
            if commercial {
                // Even redelivery/restart asks SQL for the remaining duration of the SAME interval.
                let seconds =
                    (deadline.saturating_sub(runtime_protocol::now_ms()) / 1000).clamp(1, 120);
                let reply = match commercial::usage(
                    &state,
                    &device.owner,
                    envelope.meta.operation_id,
                    "reserve",
                    seconds,
                )
                .await
                {
                    Ok(Some(reply)) => reply,
                    Ok(None) => {
                        deferred_error = Some(ApiError::forbidden());
                        continue;
                    }
                    Err(error) => {
                        deferred_error = Some(error);
                        continue;
                    }
                };
                if let Err(error) = reply.check() {
                    release_claim(&state, &id, &claim).await?;
                    deferred_error = Some(error);
                    continue;
                }
                if let Request::Execute { tool, .. } = &envelope.payload {
                    if let Err(error) = commercial::bounded(&reply, tool) {
                        abort_claim(
                            &state,
                            &device.owner,
                            &id,
                            &claim,
                            envelope.meta.operation_id,
                        )
                        .await?;
                        deferred_error = Some(error);
                        continue;
                    }
                }
                let remaining = reply.remaining_ms();
                budget_end = Some(Instant::now() + Duration::from_millis(remaining));
                deadline = deadline.min(runtime_protocol::now_ms().saturating_add(remaining));
                if let Some(saved) = persisted {
                    deadline = deadline.min(saved as u64);
                }
            }
            let connection = state.store.lock().await;
            let tx = connection
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await?;
            if state.config.commercial.is_some() && !recovery {
                tx.execute("INSERT INTO commercial_dispatches(request_id,deadline) VALUES(?1,?2) ON CONFLICT(request_id) DO NOTHING",params![id.clone(),deadline as i64]).await?;
                let row = store::first(
                    &tx,
                    "SELECT deadline FROM commercial_dispatches WHERE request_id=?1",
                    params![id.clone()],
                )
                .await?;
                deadline = deadline.min(store::integer(&row, 0)? as u64);
            }
            let mut previously_dispatched = false;
            if commercial {
                let own=tx.query("SELECT dispatched FROM commercial_dispatch_claims WHERE request_id=?1 AND token=?2 AND claim_until>?3",params![id.clone(),claim.clone(),security::now()]).await?.next().await?;
                if let Some(row) = own {
                    previously_dispatched = store::integer(&row, 0)? != 0;
                } else {
                    tx.commit().await?;
                    continue;
                }
            }
            if deadline <= runtime_protocol::now_ms()
                || budget_end.is_some_and(|end| end <= Instant::now())
            {
                tx.commit().await?;
                drop(connection);
                if commercial {
                    abort_claim(
                        &state,
                        &device.owner,
                        &id,
                        &claim,
                        envelope.meta.operation_id,
                    )
                    .await?;
                }
                continue;
            }
            if !previously_dispatched {
                envelope.meta.deadline = deadline;
            }
            let changed=tx.execute("UPDATE relay SET lease_until=?1,deadline=?2,envelope=?3 WHERE id=?4 AND result IS NULL AND lease_until<=?5 AND EXISTS(SELECT 1 FROM devices d WHERE d.id=relay.device_id AND d.revoked=0) AND EXISTS(SELECT 1 FROM grants g WHERE g.family=relay.grant_family AND g.revoked=0 AND g.expires_at>?5)",params![security::now()+30,deadline as i64,serde_json::to_string(&envelope)?,id.clone(),security::now()]).await?;
            if changed > 0 && commercial {
                tx.execute("UPDATE commercial_dispatch_claims SET dispatched=1,claim_until=0 WHERE request_id=?1 AND token=?2",params![id.clone(),claim.clone()]).await?;
            }
            tx.commit().await?;
            drop(connection);
            if changed > 0 {
                if let Some(end) = budget_end {
                    budgets.insert(id.clone(), end);
                }
                commands.push(json!({"request_id":id,"envelope":envelope}));
            } else if commercial {
                abort_claim(
                    &state,
                    &device.owner,
                    &id,
                    &claim,
                    envelope.meta.operation_id,
                )
                .await?;
            }
        }
        if commands.is_empty() {
            if let Some(error) = deferred_error {
                return Err(error);
            }
        }
        if !commands.is_empty()
            || start.elapsed() >= Duration::from_secs(state.config.poll_seconds.min(25))
        {
            if state.config.commercial.is_some() {
                for command in &mut commands {
                    let deadline = command["envelope"]["meta"]["deadline"]
                        .as_u64()
                        .unwrap_or(0);
                    let monotonic = command["request_id"]
                        .as_str()
                        .and_then(|id| budgets.get(id))
                        .map(|end| end.saturating_duration_since(Instant::now()).as_millis() as u64)
                        .unwrap_or(120000);
                    command["server_budget_ms"] = json!(
                        deadline
                            .saturating_sub(runtime_protocol::now_ms())
                            .min(120000)
                            .min(monotonic)
                    );
                }
            }
            return Ok(Json(json!({"commands":commands})));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
#[derive(Deserialize)]
pub(crate) struct AgentResult {
    request_id: uuid::Uuid,
    envelope: ResponseEnvelope,
}
pub(crate) async fn result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AgentResult>,
) -> ApiResult<Json<Value>> {
    let device = agent(&state, &headers).await?;
    let result = serde_json::to_string(&input.envelope)?;
    if result.len() > 768 * 1024 {
        return Err(ApiError::bad(
            "Resultado remoto excede 768 KiB; use leitura paginada",
        ));
    }
    let connection = state.store.lock().await;
    let tx = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;
    let mut rows=tx.query("SELECT envelope,result FROM relay WHERE id=?1 AND device_id=?2 AND user_id=?3 AND (?4=0 OR (lease_until>0 AND EXISTS(SELECT 1 FROM commercial_admissions c WHERE c.request_id=relay.id AND c.authorized=1))) AND EXISTS(SELECT 1 FROM devices WHERE id=?2 AND revoked=0)",params![input.request_id.to_string(),device.id.clone(),device.owner.clone(),i64::from(state.config.commercial.is_some())]).await?;
    let row = rows.next().await?.ok_or_else(ApiError::not_found)?;
    drop(rows);
    let expected: RequestEnvelope = serde_json::from_str(&store::text(&row, 0)?)?;
    if serde_json::to_value(&expected.meta)? != serde_json::to_value(&input.envelope.meta)? {
        return Err(ApiError::bad(
            "Identidade da resposta diverge da requisição",
        ));
    }
    let prior: Option<String> = row.get(1)?;
    if let Some(prior) = prior {
        if prior != result {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "result_conflict",
                "Resultado já registrado com conteúdo diferente",
            ));
        }
    } else {
        tx.execute(
            "UPDATE relay SET result=?1 WHERE id=?2 AND result IS NULL",
            params![result, input.request_id.to_string()],
        )
        .await?;
    }
    if let Request::Execute { tool, .. } = &expected.payload {
        let producer = match tool.as_str() {
            "process.spawn" => Some(("process", "process_id")),
            "terminal.open" => Some(("terminal", "terminal_id")),
            _ => None,
        };
        if let (Some((kind, field)), Ok(value)) = (producer, &input.envelope.payload) {
            if value["status"] == "Succeeded" && value["tool"] == *tool {
                if let Some(resource) = value["result"][field]
                    .as_str()
                    .and_then(|v| v.parse::<uuid::Uuid>().ok())
                {
                    tx.execute("INSERT INTO commercial_resources(device_id,user_id,kind,resource_id) VALUES(?1,?2,?3,?4) ON CONFLICT DO NOTHING",params![device.id.clone(),device.owner.clone(),kind,resource.to_string()]).await?;
                }
            }
        }
    }
    tx.commit().await?;
    drop(connection);
    if !recovery(&expected.payload) && terminal_result(&input.envelope) {
        commercial::usage(
            &state,
            &device.owner,
            expected.meta.operation_id,
            "complete",
            120,
        )
        .await?;
    }
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn unsupported_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let _ = oauth::grant(&state, &headers).await?;
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert("allow", "POST".parse().unwrap());
    Ok(response)
}
fn rpc_error(id: Value, code: i64, message: &str) -> Json<Value> {
    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
fn rpc_result(id: Value, result: Value) -> Json<Value> {
    Json(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
fn tool_result(value: Value, error: bool) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":error})
}
fn remote_schema(schema: &mut Value, requires_device: bool) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    for composition in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = object.get_mut(composition).and_then(Value::as_array_mut) {
            for variant in variants {
                remote_schema(variant, requires_device);
            }
        }
    }
    object.insert("type".into(), json!("object"));
    let properties = object.entry("properties").or_insert_with(|| json!({}));
    if let Some(properties) = properties.as_object_mut() {
        properties.insert(
            "device_id".into(),
            json!({"type":"string","description":"Computador autorizado opcional"}),
        );
        properties.entry("session_id").or_insert_with(||json!({"type":"string","format":"uuid","description":"Sessão operacional opcional; prefira _meta.runtimeSessionId quando a ferramenta usa este argumento"}));
        properties.insert("runtime_operation_id".into(),json!({"type":"string","format":"uuid","description":"Para retomar uma operação, repita o operation_id retornado nesta propriedade. Omita para iniciar uma operação nova."}));
    }
    if requires_device {
        let required = object.entry("required").or_insert_with(|| json!([]));
        if let Some(required) = required.as_array_mut()
            && !required.iter().any(|v| v == "device_id")
        {
            required.push(json!("device_id"));
        }
    }
}
#[derive(Deserialize)]
pub(crate) struct Rpc {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}
pub(crate) async fn mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Rpc>,
) -> ApiResult<Response> {
    let mut grant = oauth::grant(&state, &headers).await?;
    if !matches!(
        input.params["name"].as_str(),
        Some(
            "operation.status"
                | "operation.cancel"
                | "process.kill"
                | "terminal.close"
                | "operation_status"
                | "operation_cancel"
                | "process_kill"
                | "terminal_close"
        )
    ) {
        commercial::authorize(&state, &grant.user_id, uuid::Uuid::new_v4(), false, true).await?;
    }
    // A paired PC must expose its tools on the host's first discovery request.
    // Prioritize an online device with tools, or fallback to the most recently active device with tools.
    if grant.selected_device.is_none() {
        let connection = state.store.lock().await;
        let mut active_rows = connection
            .query(
                "SELECT id FROM devices WHERE user_id=?1 AND revoked=0 AND tools != '[]' AND tools != '' AND heartbeat_at>?2 ORDER BY heartbeat_at DESC LIMIT 2",
                params![grant.user_id.clone(), security::now() - 180],
            )
            .await?;
        let active_first = active_rows
            .next()
            .await?
            .map(|row| store::text(&row, 0))
            .transpose()?;
        let active_has_second = active_rows.next().await?.is_some();
        drop(active_rows);

        let target_device_id = if !active_has_second && active_first.is_some() {
            active_first
        } else {
            let mut recent = connection
                .query(
                    "SELECT id FROM devices WHERE user_id=?1 AND revoked=0 AND tools != '[]' AND tools != '' ORDER BY heartbeat_at DESC LIMIT 1",
                    params![grant.user_id.clone()],
                )
                .await?;
            let with_tools = recent
                .next()
                .await?
                .map(|row| store::text(&row, 0))
                .transpose()?;
            if with_tools.is_some() {
                with_tools
            } else {
                let mut any_recent = connection
                    .query(
                        "SELECT id FROM devices WHERE user_id=?1 AND revoked=0 ORDER BY heartbeat_at DESC LIMIT 1",
                        params![grant.user_id.clone()],
                    )
                    .await?;
                any_recent
                    .next()
                    .await?
                    .map(|row| store::text(&row, 0))
                    .transpose()?
            }
        };

        if let Some(id) = target_device_id {
            connection
                .execute(
                    "UPDATE grants SET selected_device=?1 WHERE id=?2 AND revoked=0",
                    params![id.clone(), grant.id.clone()],
                )
                .await?;
            grant.selected_device = Some(id);
        }
    }
    if let Some(version) = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
        && !["2025-03-26", "2025-06-18", "2025-11-25"].contains(&version)
    {
        return Err(ApiError::bad("Versão MCP não suportada"));
    }
    if let Some(accept) = headers.get("accept").and_then(|v| v.to_str().ok())
        && !accept.contains("application/json")
        && !accept.contains("*/*")
    {
        return Err(ApiError::new(
            StatusCode::NOT_ACCEPTABLE,
            "not_acceptable",
            "Aceite application/json",
        ));
    }
    let id = input.id.clone().unwrap_or(Value::Null);
    if input.jsonrpc != "2.0"
        || input.method.len() > 128
        || input
            .id
            .as_ref()
            .is_some_and(|v| !v.is_string() && !v.is_number())
    {
        return Ok(rpc_error(id, -32600, "Requisição JSON-RPC inválida").into_response());
    }
    if input.id.is_none() {
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    state.rate(format!("mcp:{}", grant.family), 120).await?;
    let output = match input.method.as_str() {
        "initialize" => {
            let requested = input.params["protocolVersion"]
                .as_str()
                .unwrap_or("2025-11-25");
            let protocol = if ["2025-03-26", "2025-06-18", "2025-11-25"].contains(&requested) {
                requested
            } else {
                "2025-11-25"
            };
            json!({"protocolVersion":protocol,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"remote-desktop-commander","version":env!("CARGO_PKG_VERSION")},"instructions":"Use device.list e device.select para escolher seu computador. Ferramentas remotas mantêm aprovações locais. Para retomar operações informe runtime_operation_id com o operation_id retornado. Use session_id para continuidade quando não for um parâmetro local da ferramenta. Resultados pendentes não significam falha da operação."})
        }
        "ping" => json!({}),
        "tools/list" => {
            oauth::require_scope(&state, &grant, "tools:read")?;
            let mut tools = vec![
                json!({"name":"device.list","description":"Lista seus computadores pareados e presença recente","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}}),
                json!({"name":"device.select","description":"Escolhe o computador desta conexão autorizada","inputSchema":{"type":"object","properties":{"device_id":{"type":"string","format":"uuid"}},"required":["device_id"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}}),
            ];
            let catalogs = if let Some(id) = grant.selected_device.as_deref() {
                let dev = owned_device(&state, &grant.user_id, id).await.ok();
                if let Some(dev) = dev && !dev.tools.is_empty() {
                    vec![dev.tools]
                } else {
                    let connection = state.store.lock().await;
                    let mut rows = connection
                        .query(
                            "SELECT tools FROM devices WHERE user_id=?1 AND revoked=0 AND tools != '[]' AND tools != '' ORDER BY heartbeat_at DESC LIMIT 20",
                            params![grant.user_id.clone()],
                        )
                        .await?;
                    let mut catalogs = Vec::new();
                    while let Some(row) = rows.next().await? {
                        if let Ok(text) = store::text(&row, 0) {
                            if let Ok(device_tools) = serde_json::from_str::<Vec<ToolDescriptor>>(&text) {
                                catalogs.push(device_tools);
                            }
                        }
                    }
                    catalogs
                }
            } else {
                let connection = state.store.lock().await;
                let mut rows = connection
                    .query(
                        "SELECT tools FROM devices WHERE user_id=?1 AND revoked=0 AND tools != '[]' AND tools != '' ORDER BY heartbeat_at DESC LIMIT 20",
                        params![grant.user_id.clone()],
                    )
                    .await?;
                let mut catalogs = Vec::new();
                while let Some(row) = rows.next().await? {
                    if let Ok(text) = store::text(&row, 0) {
                        if let Ok(device_tools) = serde_json::from_str::<Vec<ToolDescriptor>>(&text) {
                            catalogs.push(device_tools);
                        }
                    }
                }
                catalogs
            };
            let mut combined = std::collections::BTreeMap::<String, ToolDescriptor>::new();
            for catalog in catalogs {
                for tool in catalog {
                    if let Some(old) = combined.get_mut(&tool.name) {
                        if old.input_schema != tool.input_schema {
                            old.input_schema =
                                json!({"anyOf":[old.input_schema.clone(),tool.input_schema]});
                        }
                        if tool.risk != "read" {
                            old.risk = tool.risk;
                        }
                    } else {
                        combined.insert(tool.name.clone(), tool);
                    }
                }
            }
            for tool in combined.into_values() {
                let mut schema = tool.input_schema.clone();
                remote_schema(&mut schema, grant.selected_device.is_none());
                tools.push(json!({"name":tool.name,"description":tool.description,"inputSchema":schema,"annotations":{"readOnlyHint":tool.risk=="read","destructiveHint":tool.risk!="read","openWorldHint":tool.risk=="high"}}));
            }
            json!({"tools":tools})
        }
        "tools/call" => {
            let Some(name) = input.params["name"].as_str() else {
                return Ok(rpc_error(id, -32602, "Nome da ferramenta ausente").into_response());
            };
            let args = input
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !args.is_object() {
                return Ok(rpc_error(id, -32602, "Argumentos devem ser objeto").into_response());
            }
            let normalized_name = name.replace('_', ".");
            match name {
                "device.list" | "device_list" => {
                    oauth::require_scope(&state, &grant, "devices:read")?;
                    tool_result(
                        json!({"devices":pairing::devices(&state,&grant.user_id).await?}),
                        false,
                    )
                }
                "device.select" | "device_select" => {
                    oauth::require_scope(&state, &grant, "devices:read")?;
                    let device_id = args["device_id"]
                        .as_str()
                        .ok_or_else(|| ApiError::bad("Informe device_id"))?;
                    let device = owned_device(&state, &grant.user_id, device_id).await?;
                    let connection = state.store.lock().await;
                    let changed=connection.execute("UPDATE grants SET selected_device=?1 WHERE id=?2 AND revoked=0 AND EXISTS(SELECT 1 FROM devices WHERE id=?1 AND user_id=?3 AND revoked=0)",params![device.id.clone(),grant.id.clone(),grant.user_id.clone()]).await?;
                    if changed == 0 {
                        return Err(ApiError::unauthorized());
                    }
                    tool_result(
                        json!({"device_id":device.id,"workspace_id":device.workspace.to_string(),"message":"Computador selecionado com sucesso"}),
                        false,
                    )
                }
                _ => {
                    oauth::require_scope(&state, &grant, "tools:execute")?;
                    let Some(device_id) = args["device_id"]
                        .as_str()
                        .or(grant.selected_device.as_deref())
                    else {
                        return Ok(rpc_result(
                            id,
                            tool_result(
                                json!({"error":"Escolha um computador com device.select"}),
                                true,
                            ),
                        )
                        .into_response());
                    };
                    let device = owned_device(&state, &grant.user_id, device_id).await?;
                    let target_tool = device.tools.iter().find(|t| {
                        t.name == name
                            || t.name == normalized_name
                            || t.name.replace('.', "_") == name
                    });
                    let Some(matched_tool) = target_tool else {
                        return Ok(rpc_error(
                            id,
                            -32602,
                            "Ferramenta ausente do catálogo deste computador",
                        )
                        .into_response());
                    };
                    let canonical_name = matched_tool.name.clone();
                    let _permit = state
                        .calls
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| ApiError::quota())?;
                    let value = forward(&state, &grant, &device, &input, &canonical_name, args).await?;
                    let error = value.get("error").is_some_and(|v| !v.is_null());
                    tool_result(value, error)
                }
            }
        }
        _ => return Ok(rpc_error(id, -32601, "Método não suportado").into_response()),
    };
    Ok(rpc_result(id, output).into_response())
}
async fn forward(
    state: &AppState,
    grant: &oauth::Grant,
    device: &Device,
    rpc: &Rpc,
    tool: &str,
    mut input: Value,
) -> ApiResult<Value> {
    let is_recovery = recovery_tool(tool);
    if is_recovery && state.config.commercial.is_some() {
        let connection = state.store.lock().await;
        if matches!(tool, "operation.status" | "operation.cancel") {
            let target = input["operation_id"]
                .as_str()
                .and_then(|v| v.parse::<uuid::Uuid>().ok())
                .ok_or_else(ApiError::forbidden)?;
            store::first(&connection,"SELECT id FROM relay WHERE device_id=?1 AND user_id=?2 AND operation_id=?3 AND lease_until>0",params![device.id.clone(),grant.user_id.clone(),target.to_string()]).await.map_err(|_|ApiError::forbidden())?;
        } else {
            let (kind, field) = if tool == "process.kill" {
                ("process", "process_id")
            } else {
                ("terminal", "terminal_id")
            };
            let target = input[field]
                .as_str()
                .and_then(|v| v.parse::<uuid::Uuid>().ok())
                .ok_or_else(ApiError::forbidden)?;
            store::first(&connection,"SELECT resource_id FROM commercial_resources WHERE device_id=?1 AND user_id=?2 AND kind=?3 AND resource_id=?4",params![device.id.clone(),grant.user_id.clone(),kind,target.to_string()]).await.map_err(|_|ApiError::forbidden())?;
        }
    } else if let Some(reply) =
        commercial::usage(state, &grant.user_id, uuid::Uuid::new_v4(), "inspect", 120).await?
    {
        reply.check()?;
        commercial::bounded(&reply, tool)?;
    }
    let local_session_argument = device
        .tools
        .iter()
        .find(|descriptor| descriptor.name == tool)
        .is_some_and(|descriptor| {
            descriptor.input_schema["properties"]
                .get("session_id")
                .is_some()
        });
    let session = rpc.params["_meta"]["runtimeSessionId"]
        .as_str()
        .or_else(|| {
            if local_session_argument {
                None
            } else {
                input["session_id"].as_str()
            }
        })
        .map(|v| {
            v.parse::<uuid::Uuid>()
                .map_err(|_| ApiError::bad("session_id inválido"))
        })
        .transpose()?;
    let parse_operation = |value: &Value| {
        value
            .as_str()
            .ok_or_else(|| ApiError::bad("Identidade da operação deve ser UUID"))
            .and_then(|v| {
                v.parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::bad("Identidade da operação deve ser UUID"))
            })
    };
    let meta_operation = rpc.params["_meta"]
        .get("runtimeOperationId")
        .map(parse_operation)
        .transpose()?;
    let argument_operation = input
        .get("runtime_operation_id")
        .map(parse_operation)
        .transpose()?;
    if meta_operation.is_some()
        && argument_operation.is_some()
        && meta_operation != argument_operation
    {
        return Err(ApiError::bad("Identidades da operação conflitantes"));
    }
    let explicit = meta_operation.or(argument_operation);
    let object = input
        .as_object_mut()
        .ok_or_else(|| ApiError::bad("Argumentos inválidos"))?;
    object.remove("device_id");
    object.remove("runtime_operation_id");
    if !local_session_argument {
        object.remove("session_id");
    }
    if state.config.commercial.is_some() {
        let schema = &device
            .tools
            .iter()
            .find(|descriptor| descriptor.name == tool)
            .ok_or_else(ApiError::not_found)?
            .input_schema;
        let validator = jsonschema::options()
            .should_validate_formats(true)
            .build(schema)
            .map_err(|_| ApiError::bad("Esquema remoto inválido"))?;
        if !validator.is_valid(&input) {
            return Err(ApiError::bad("Argumentos remotos inválidos"));
        }
    }
    let payload = Request::Execute {
        tool: tool.to_owned(),
        input,
    };
    let mut meta = Metadata::new(Some(device.workspace), 120_000);
    meta.session_id = session;
    if let Some(operation) = explicit {
        meta.operation_id = operation;
    }
    // JSON-RPC ids are correlation only and may be reused by separate chats.
    // Only an explicitly retained runtimeOperationId identifies a durable retry.
    let dedupe = security::hash(&format!(
        "{}:{}:{}",
        grant.family, device.id, meta.operation_id
    ));
    let envelope = RequestEnvelope { meta, payload };
    let serialized = serde_json::to_string(&envelope)?;
    if serialized.len() > 512 * 1024 {
        return Err(ApiError::bad("Requisição remota excede 512 KiB"));
    }
    let (request_id, _execute, _operation) = {
        let connection = state.store.lock().await;
        let tx = connection
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        let mut live=tx.query("SELECT id FROM devices WHERE id=?1 AND user_id=?2 AND revoked=0 AND (?5=0 OR heartbeat_at>?4-90) AND EXISTS(SELECT 1 FROM grants WHERE id=?3 AND revoked=0 AND expires_at>?4)",params![device.id.clone(),grant.user_id.clone(),grant.id.clone(),security::now(),i64::from(state.config.commercial.is_some())]).await?;
        if live.next().await?.is_none() {
            return Err(ApiError::unauthorized());
        }
        drop(live);
        tx.execute(
            "DELETE FROM relay WHERE device_id=?1 AND created_at<?2",
            params![device.id.clone(), security::now() - 86400],
        )
        .await?;
        let mut existing=tx.query("SELECT id,envelope FROM relay WHERE device_id=?1 AND user_id=?2 AND (dedupe_key=?3 OR operation_id=?4)",params![device.id.clone(),grant.user_id.clone(),dedupe.clone(),envelope.meta.operation_id.to_string()]).await?;
        let old = existing.next().await?;
        drop(existing);
        let (id, execute, operation) = if let Some(row) = old {
            let previous: RequestEnvelope = serde_json::from_str(&store::text(&row, 1)?)?;
            if serde_json::to_value(&previous.payload)? != serde_json::to_value(&envelope.payload)?
                || previous.meta.session_id != envelope.meta.session_id
            {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "operation_conflict",
                    "Identidade reutilizada com comando diferente",
                ));
            }
            let id = store::text(&row, 0)?;
            let mut admission = tx
                .query(
                    "SELECT authorized FROM commercial_admissions WHERE request_id=?1",
                    params![id.clone()],
                )
                .await?;
            let authorized = admission
                .next()
                .await?
                .map(|r| store::integer(&r, 0))
                .transpose()?
                .unwrap_or(0)
                != 0;
            drop(admission);
            (
                id,
                !authorized && previous.meta.deadline > runtime_protocol::now_ms(),
                previous.meta.operation_id,
            )
        } else {
            let mut count=tx.query("SELECT COUNT(*),COALESCE(SUM(CASE WHEN result IS NULL AND deadline>?2 THEN 1 ELSE 0 END),0) FROM relay WHERE device_id=?1",params![device.id.clone(),runtime_protocol::now_ms()as i64]).await?;
            let row = count.next().await?.ok_or_else(ApiError::not_found)?;
            drop(count);
            if store::integer(&row, 0)?
                >= state.config.result_limit + if is_recovery { 16 } else { 0 }
                || store::integer(&row, 1)?
                    >= state.config.queue_limit + if is_recovery { 16 } else { 0 }
            {
                return Err(ApiError::quota());
            }
            let id = envelope.meta.request_id.to_string();
            tx.execute("INSERT INTO relay(id,device_id,user_id,grant_family,operation_id,dedupe_key,envelope,created_at,deadline) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id.clone(),device.id.clone(),grant.user_id.clone(),grant.family.clone(),envelope.meta.operation_id.to_string(),dedupe,serialized,security::now(),envelope.meta.deadline as i64]).await?;
            if state.config.commercial.is_some() {
                tx.execute(
                    "INSERT INTO commercial_admissions(request_id) VALUES(?1)",
                    params![id.clone()],
                )
                .await?;
            }
            (id, true, envelope.meta.operation_id)
        };
        tx.commit().await?;
        (id, execute, operation)
    };
    // Enqueue is an inspection only; the poll reserves this persisted operation identity.
    if state.config.commercial.is_some() {
        let connection = state.store.lock().await;
        let changed=connection.execute("UPDATE commercial_admissions SET authorized=1 WHERE request_id=?1 AND EXISTS(SELECT 1 FROM relay r JOIN devices d ON d.id=r.device_id WHERE r.id=?1 AND d.revoked=0 AND EXISTS(SELECT 1 FROM grants g WHERE g.id=?2 AND g.revoked=0 AND g.expires_at>?3))",params![request_id.clone(),grant.id.clone(),security::now()]).await?;
        if changed == 0 {
            return Err(ApiError::forbidden());
        }
    }
    let start = Instant::now();
    loop {
        let (saved, envelope) = {
            let connection = state.store.lock().await;
            let row=store::first(&connection,"SELECT r.result,r.envelope FROM relay r JOIN devices d ON d.id=r.device_id JOIN grants g ON g.id=?3 WHERE r.id=?1 AND r.user_id=?2 AND d.revoked=0 AND g.revoked=0 AND g.expires_at>?4",params![request_id.clone(),grant.user_id.clone(),grant.id.clone(),security::now()]).await?;
            (
                row.get::<Option<String>>(0)?,
                serde_json::from_str::<RequestEnvelope>(&store::text(&row, 1)?)?,
            )
        };
        if let Some(saved) = saved {
            let response: ResponseEnvelope = serde_json::from_str(&saved)?;
            return Ok(match response.payload {
                Ok(value) => {
                    json!({"operation_id":response.meta.operation_id,"request_id":request_id,"result":value})
                }
                Err(error) => {
                    json!({"operation_id":response.meta.operation_id,"request_id":request_id,"error":error})
                }
            });
        }
        if envelope.meta.deadline <= runtime_protocol::now_ms() {
            return Ok(
                json!({"status":"deadline_elapsed","request_id":request_id,"operation_id":envelope.meta.operation_id,"error":{"code":"delivery_deadline_elapsed","message":"Prazo de entrega expirou. A execução pode ter começado no computador; consulte a operação antes de usar uma nova identidade."}}),
            );
        }
        if start.elapsed() >= Duration::from_secs(state.config.call_seconds.min(55)) {
            return Ok(
                json!({"status":"pending","request_id":request_id,"operation_id":envelope.meta.operation_id,"message":"A operação pode continuar no computador. Repita o mesmo comando com runtime_operation_id igual ao operation_id retornado para consultar o resultado sem criar outra operação."}),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn recovery_tool(tool: &str) -> bool {
    matches!(
        tool,
        "operation.status" | "operation.cancel" | "process.kill" | "terminal.close"
    )
}
fn recovery(payload: &Request) -> bool {
    matches!(payload,Request::Execute{tool,..} if recovery_tool(tool))
}

fn terminal_result(reply: &ResponseEnvelope) -> bool {
    reply.payload.as_ref().ok().is_some_and(|value| {
        matches!(
            value["status"].as_str(),
            Some("Succeeded" | "Failed" | "Cancelled" | "TimedOut")
        )
    })
}

async fn release_claim(state: &AppState, id: &str, token: &str) -> ApiResult<()> {
    let connection = state.store.lock().await;
    connection
        .execute(
            "UPDATE commercial_dispatch_claims SET claim_until=0 WHERE request_id=?1 AND token=?2",
            params![id, token],
        )
        .await?;
    Ok(())
}
async fn abort_claim(
    state: &AppState,
    owner: &str,
    id: &str,
    token: &str,
    operation: uuid::Uuid,
) -> ApiResult<()> {
    // Permanently fence this never-dispatched job BEFORE HTTP. A suspended abort
    // must never cancel a claim reclaimed and delivered by another poll.
    let safe = {
        let connection = state.store.lock().await;
        connection.execute("UPDATE commercial_dispatch_claims SET claim_until=9223372036854775807 WHERE request_id=?1 AND token=?2 AND claim_until>?3 AND dispatched=0",params![id,token,security::now()]).await?>0
    };
    if safe {
        commercial::usage(state, owner, operation, "abort", 120).await?;
    }
    Ok(())
}
