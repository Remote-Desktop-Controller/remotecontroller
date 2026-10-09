use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use libsql::params;
use remote_service::CommercialConfig;
use remote_service::{Config, application};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, AtomicUsize, Ordering},
};
use tower::ServiceExt;
const SECRET: &str = "commercial-fixture-secret-at-least-32-chars";
const SUBJECT: &str = "b00f9567-f927-4d4a-a9e0-32c5b899a8f4";
const ORIGIN: &str = "http://127.0.0.1:8787";

#[derive(Default)]
struct Portal {
    executions: AtomicUsize,
    completions: AtomicUsize,
    aborts: AtomicUsize,
    lease_ms: AtomicI64,
    expiry_shift: AtomicI64,
    reserved: Mutex<std::collections::HashSet<String>>,
    free: AtomicUsize,
    usage_calls: Mutex<Vec<Value>>,
    inspections: AtomicUsize,
    exchanges: AtomicUsize,
    expiry: AtomicI64,
    override_identity: Mutex<Option<Value>>,
    mismatch: AtomicUsize,
    deny_execution: AtomicUsize,
    gate_mode: AtomicUsize,
    gate_started: tokio::sync::Notify,
    gate_continue: tokio::sync::Notify,
    tickets: Mutex<std::collections::HashSet<String>>,
}
fn now() -> i64 {
    remote_service::security::now()
}
async fn exchange(
    State(portal): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(headers["authorization"], format!("Bearer {SECRET}"));
    assert_eq!(
        input["redirect_uri"],
        format!("{ORIGIN}/auth/commercial/callback")
    );
    portal.exchanges.fetch_add(1, Ordering::SeqCst);
    if !portal
        .tickets
        .lock()
        .unwrap()
        .insert(input["ticket"].as_str().unwrap().to_owned())
    {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let identity=portal.override_identity.lock().unwrap().clone().unwrap_or(json!({"subject":SUBJECT,"email":"commercial@example.com","email_verified":true,"audience":ORIGIN}));
    (StatusCode::OK, Json(identity))
}
async fn authorize(
    State(portal): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Json<Value> {
    assert_eq!(headers["authorization"], format!("Bearer {SECRET}"));
    assert_eq!(input["subject"], SUBJECT);
    uuid::Uuid::parse_str(input["operation_id"].as_str().unwrap()).unwrap();
    if input["mode"] == "execute" {
        portal.executions.fetch_add(1, Ordering::SeqCst);
        if portal.deny_execution.load(Ordering::SeqCst) == 0 {
            portal
                .expiry
                .compare_exchange(0, now() + 2592000, Ordering::SeqCst, Ordering::SeqCst)
                .ok();
        }
    } else {
        portal.inspections.fetch_add(1, Ordering::SeqCst);
    }
    let expiry = portal.expiry.load(Ordering::SeqCst);
    if portal.gate_mode.load(Ordering::SeqCst) == (if input["mode"] == "execute" { 1 } else { 2 }) {
        portal.gate_started.notify_one();
        portal.gate_continue.notified().await;
    }
    let allowed = expiry > now();
    Json(
        json!({"subject":if portal.mismatch.load(Ordering::SeqCst)==1 {"forged"} else {SUBJECT},"operation_id":if portal.mismatch.load(Ordering::SeqCst)==2 {"forged"} else {input["operation_id"].as_str().unwrap()},"allowed":allowed,"state":if expiry==0 {"READY_TO_ACTIVATE"}else if allowed{"ACTIVE"}else{"EXPIRED"},"expires_at":if expiry==0 {None}else{Some(expiry)}}),
    )
}
async fn usage(
    State(portal): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Json<Value> {
    assert_eq!(headers["authorization"], format!("Bearer {SECRET}"));
    portal.usage_calls.lock().unwrap().push(input.clone());
    if input["action"] == "complete" {
        portal.completions.fetch_add(1, Ordering::SeqCst);
    }
    if input["action"] == "abort" {
        portal.aborts.fetch_add(1, Ordering::SeqCst);
    }
    let reserve = input["action"] == "reserve";
    if reserve
        && portal
            .reserved
            .lock()
            .unwrap()
            .insert(input["operation_id"].as_str().unwrap().to_owned())
    {
        portal.executions.fetch_add(1, Ordering::SeqCst);
    } else {
        portal.inspections.fetch_add(1, Ordering::SeqCst);
    }
    if portal.gate_mode.load(Ordering::SeqCst) == if reserve { 1 } else { 2 } {
        portal.gate_started.notify_one();
        portal.gate_continue.notified().await;
    }
    let expiry = portal.expiry.load(Ordering::SeqCst);
    let allowed = (expiry == 0 || expiry > now())
        && (!reserve || portal.deny_execution.load(Ordering::SeqCst) == 0);
    Json(
        json!({"subject": if portal.mismatch.load(Ordering::SeqCst)==1 {"forged"} else {SUBJECT},
    "operation_id": if portal.mismatch.load(Ordering::SeqCst)==2 {json!("forged")} else {input["operation_id"].clone()},
    "allowed":allowed,"state":if allowed {"ACTIVE"} else {"EXHAUSTED"},"plan":if portal.free.load(Ordering::SeqCst)==1 {"FREE"} else {"PREMIUM"},
    "expires_at":if allowed {Some((now()+input["max_seconds"].as_i64().unwrap_or(120)).min(if expiry>0 {expiry} else {i64::MAX})+portal.expiry_shift.load(Ordering::SeqCst))} else {None},
    "lease_remaining_ms":if portal.lease_ms.load(Ordering::SeqCst)>0 {portal.lease_ms.load(Ordering::SeqCst)}else{120000},"remaining_seconds":if allowed {1800} else {0},"resets_at":now()+86400,"error_code":if allowed {None} else {Some("FREE_DAILY_LIMIT_REACHED")}}),
    )
}
struct Fixture {
    dir: tempfile::TempDir,
    app: Router,
    portal: Arc<Portal>,
    task: tokio::task::JoinHandle<()>,
    config: Config,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture() -> Fixture {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("remote_service=debug")
        .try_init();
    let portal = Arc::new(Portal::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/api/mcp/exchange", post(exchange))
        .route("/api/mcp/authorize", post(authorize))
        .route("/api/mcp/usage", post(usage))
        .with_state(portal.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = Config::new(ORIGIN.into(), true, vec![]).unwrap();
    config.commercial =
        Some(CommercialConfig::new(format!("http://{address}"), SECRET.into(), true).unwrap());
    config.poll_seconds = 0;
    config.call_seconds = 0;
    config.queue_limit = 1;
    let dir = tempfile::tempdir().unwrap();
    let app = application(&dir.path().join("service.db"), config.clone())
        .await
        .unwrap();
    Fixture {
        dir,
        app,
        portal,
        task,
        config,
    }
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: Option<&str>,
    bearer: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(bearer) = bearer {
        builder = builder.header("authorization", format!("Bearer {bearer}"));
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
async fn begin(app: &Router) -> (String, String) {
    let response = request(
        app,
        "GET",
        "/login?next=%2Foauth%2Fauthorize%3Fclient_id%3Dtest",
        Value::Null,
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let url = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(url.path(), "/connect");
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    (cookie, state)
}
async fn callback(
    app: &Router,
    cookie: &str,
    state: &str,
    ticket: &str,
) -> axum::response::Response {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", state)
        .append_pair("ticket", ticket)
        .finish();
    request(
        app,
        "GET",
        &format!("/auth/commercial/callback?{query}"),
        Value::Null,
        Some(cookie),
        None,
    )
    .await
}
async fn login(f: &Fixture) -> String {
    let (cookie, state) = begin(&f.app).await;
    let response = callback(&f.app, &cookie, &state, &uuid::Uuid::new_v4().to_string()).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            v.to_str()
                .ok()
                .filter(|v| v.starts_with("rdc_session="))
                .map(|v| v.split(';').next().unwrap().to_owned())
        })
        .unwrap()
}
async fn db(f: &Fixture) -> (libsql::Database, libsql::Connection) {
    let database = libsql::Builder::new_local(f.dir.path().join("service.db"))
        .build()
        .await
        .unwrap();
    let connection = database.connect().unwrap();
    (database, connection)
}
// Seed an operational OAuth grant and owned device; identity itself always comes through HTTP exchange.
async fn operational(f: &Fixture) -> String {
    login(f).await;
    let (_database, conn) = db(f).await;
    let row = conn
        .query(
            "SELECT user_id FROM commercial_accounts WHERE subject=?1",
            params![SUBJECT],
        )
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    let owner: String = row.get(0).unwrap();
    conn.execute(
        "INSERT INTO clients VALUES('client','Fixture','[]',?1)",
        params![now()],
    )
    .await
    .unwrap();
    conn.execute("INSERT INTO grants(id,family,user_id,client_id,access_hash,refresh_hash,resource,scope,expires_at,refresh_expires_at,selected_device) VALUES('grant','family',?1,'client',?2,'unused',?3,'tools:read tools:execute devices:read',?4,?4,'device')",params![owner.clone(),remote_service::security::hash("access"),format!("{ORIGIN}/mcp"),now()+86400]).await.unwrap();
    let tools = json!([{"name":"test.echo","version":"1.0.0","description":"Echo fixture","capabilities":[],"risk":"read","input_schema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false},"output_schema":null}]);
    conn.execute("INSERT INTO devices(id,user_id,token_hash,name,workspace_id,tools,heartbeat_at) VALUES('device',?1,?2,'Fixture',?3,?4,?5)",params![owner.clone(),remote_service::security::hash("agent"),uuid::Uuid::new_v4().to_string(),tools.to_string(),now()]).await.unwrap();
    owner
}
async fn rpc(f: &Fixture, name: &str, args: Value) -> axum::response::Response {
    request(&f.app,"POST","/mcp",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}),None,Some("access")).await
}
async fn json_response(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn commercial_configuration_requires_trusted_origin_and_secret() {
    for origin in [
        "http://portal.example",
        "https://user:password@portal.example",
        "https://portal.example/path",
        "https://portal.example?secret=x",
        "https://portal.example#fragment",
    ] {
        assert!(
            CommercialConfig::new(origin.into(), SECRET.into(), false).is_err(),
            "{origin}"
        );
    }
    assert!(CommercialConfig::new("https://portal.example".into(), "short".into(), false).is_err());
    assert!(CommercialConfig::new("http://127.0.0.1:9000".into(), SECRET.into(), false).is_err());
    assert!(CommercialConfig::new("http://127.0.0.1:9000".into(), SECRET.into(), true).is_ok());
}
#[tokio::test]
async fn inactive_idle_agent_stays_connected_without_provider_polling_or_activation() {
    let mut f = fixture().await;
    operational(&f).await;
    f.config.poll_seconds = 1;
    f.app = application(&f.dir.path().join("service.db"), f.config.clone())
        .await
        .unwrap();
    for expiry in [0, now() - 1] {
        f.portal.expiry.store(expiry, Ordering::SeqCst);
        let response = request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_response(response).await["commands"], json!([]));
    }
    assert_eq!(f.portal.inspections.load(Ordering::SeqCst), 0);
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn expired_pending_delivery_is_recoverable_and_can_resume_after_renewal() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    f.portal.expiry.store(now() - 1, Ordering::SeqCst);
    let denied = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::PAYMENT_REQUIRED);
    let (_database, conn) = db(&f).await;
    let row = conn
        .query("SELECT lease_until FROM relay", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 0);
    f.portal.expiry.store(now() + 2592000, Ordering::SeqCst);
    let response = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_response(response).await["commands"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn nonce_cookie_and_ticket_replay_are_bound() {
    let f = fixture().await;
    let (cookie, state) = begin(&f.app).await;
    assert_eq!(
        callback(&f.app, &cookie, "wrong", "one").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.portal.exchanges.load(Ordering::SeqCst), 0);
    assert_eq!(
        callback(&f.app, &cookie, &state, "one").await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        callback(&f.app, &cookie, &state, "one").await.status(),
        StatusCode::UNAUTHORIZED
    );
    let (cookie, state) = begin(&f.app).await;
    assert_eq!(
        callback(&f.app, &cookie, &state, "one").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(f.portal.expiry.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn identity_is_verified_and_conflicting_local_email_never_taken_over() {
    let f = fixture().await;
    for override_value in [
        json!({"subject":SUBJECT,"email":"commercial@example.com","email_verified":false,"audience":ORIGIN}),
        json!({"subject":SUBJECT,"email":"commercial@example.com","email_verified":true,"audience":"https://evil.example"}),
        json!({"subject":"bad","email":"commercial@example.com","email_verified":true,"audience":ORIGIN}),
    ] {
        *f.portal.override_identity.lock().unwrap() = Some(override_value);
        let (cookie, state) = begin(&f.app).await;
        assert_eq!(
            callback(&f.app, &cookie, &state, &uuid::Uuid::new_v4().to_string())
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    *f.portal.override_identity.lock().unwrap() = None;
    let (_database, conn) = db(&f).await;
    conn.execute(
        "INSERT INTO users VALUES('legacy','commercial@example.com','legacy-hash',?1)",
        params![now()],
    )
    .await
    .unwrap();
    let (cookie, state) = begin(&f.app).await;
    assert_eq!(
        callback(&f.app, &cookie, &state, "conflict").await.status(),
        StatusCode::CONFLICT
    );
    let rows = conn
        .query("SELECT COUNT(*) FROM commercial_accounts", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rows.get::<i64>(0).unwrap(), 0);
}
#[tokio::test]
async fn setup_and_invalid_commands_do_not_activate_first_remote_call_does() {
    let f = fixture().await;
    operational(&f).await;
    for method in ["initialize", "tools/list"] {
        let response = request(
            &f.app,
            "POST",
            "/mcp",
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":{}}),
            None,
            Some("access"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(
        rpc(&f, "test.echo", json!({})).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":5})).await.status(),
        StatusCode::BAD_REQUEST
    );
    rpc(&f, "absent", json!({})).await;
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
    assert_eq!(f.portal.expiry.load(Ordering::SeqCst), 0);
    let response = rpc(&f, "test.echo", json!({"text":"hello"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = json_response(response).await;
    let operation = response["result"]["structuredContent"]["operation_id"]
        .as_str()
        .unwrap();
    let expiry = f.portal.expiry.load(Ordering::SeqCst);
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
    assert_eq!(
        rpc(
            &f,
            "test.echo",
            json!({"text":"hello","runtime_operation_id":operation})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"new"})).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    login(&f).await;
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
    assert_eq!(f.portal.expiry.load(Ordering::SeqCst), expiry);
    f.portal.expiry.store(now() - 1, Ordering::SeqCst);
    assert_eq!(
        rpc(
            &f,
            "test.echo",
            json!({"text":"hello","runtime_operation_id":operation})
        )
        .await
        .status(),
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent")
        )
        .await
        .status(),
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn direct_mcp_old_identity_and_local_auth_cannot_bypass() {
    let f = fixture().await;
    let owner = operational(&f).await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/auth/login",
            json!({"email":"commercial@example.com","password":"a sufficiently long password"}),
            None,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/auth/register",
            json!({"email":"attacker@example.com","password":"a sufficiently long password"}),
            None,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let (_database, conn) = db(&f).await;
    conn.execute(
        "DELETE FROM commercial_accounts WHERE user_id=?1",
        params![owner],
    )
    .await
    .unwrap();
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello","subject":SUBJECT}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn mismatched_or_unavailable_portal_fails_closed() {
    let f = fixture().await;
    operational(&f).await;
    for mismatch in [1, 2] {
        f.portal.mismatch.store(mismatch, Ordering::SeqCst);
        assert_eq!(
            rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    f.task.abort();
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn unavailable_service_reservations_are_never_delivered_or_reactivated_after_deadline() {
    let f = fixture().await;
    operational(&f).await;
    f.portal.deny_execution.store(1, Ordering::SeqCst);
    let operation = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        rpc(
            &f,
            "test.echo",
            json!({"text":"hello","runtime_operation_id":operation})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent")
        )
        .await
        .status(),
        StatusCode::PAYMENT_REQUIRED
    );
    let (_database, conn) = db(&f).await;
    let row = conn
        .query("SELECT id,envelope FROM relay", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    let id: String = row.get(0).unwrap();
    let mut envelope: Value = serde_json::from_str(&row.get::<String>(1).unwrap()).unwrap();
    envelope["meta"]["deadline"] = json!(runtime_protocol::now_ms() - 1);
    conn.execute(
        "UPDATE relay SET envelope=?1,deadline=?2",
        params![envelope.to_string(), runtime_protocol::now_ms() as i64 - 1],
    )
    .await
    .unwrap();
    f.portal.deny_execution.store(0, Ordering::SeqCst);
    assert_eq!(
        json_response(
            request(
                &f.app,
                "GET",
                "/api/agent/poll",
                Value::Null,
                None,
                Some("agent")
            )
            .await
        )
        .await["commands"],
        json!([])
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 1);
    let result = runtime_protocol::ResponseEnvelope {
        meta: serde_json::from_value(envelope["meta"].clone()).unwrap(),
        payload: Ok(json!({"completed":true})),
    };
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/agent/result",
            json!({"request_id":id,"envelope":result}),
            None,
            Some("agent")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn started_command_can_finish_after_expiry_but_gateway_cannot_read_or_deliver_more() {
    let f = fixture().await;
    operational(&f).await;
    let response = rpc(&f, "test.echo", json!({"text":"hello"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_response(response).await;
    let operation = body["result"]["structuredContent"]["operation_id"]
        .as_str()
        .unwrap();
    let response = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let commands = json_response(response).await;
    let command = &commands["commands"][0];
    assert!(command.is_object());
    f.portal.expiry.store(now() - 1, Ordering::SeqCst);
    let response=request(&f.app,"POST","/api/agent/result",json!({"request_id":command["request_id"],"envelope":{"meta":command["envelope"]["meta"],"payload":{"Ok":{"completed":true}}}}),None,Some("agent")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        rpc(
            &f,
            "test.echo",
            json!({"text":"hello","runtime_operation_id":operation})
        )
        .await
        .status(),
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn stale_device_or_insufficient_scope_does_not_activate() {
    let f = fixture().await;
    operational(&f).await;
    let (_database, conn) = db(&f).await;
    conn.execute("UPDATE devices SET heartbeat_at=?1", params![now() - 100])
        .await
        .unwrap();
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::UNAUTHORIZED
    );
    conn.execute("UPDATE devices SET heartbeat_at=?1", params![now()])
        .await
        .unwrap();
    conn.execute("UPDATE grants SET scope='tools:read'", ())
        .await
        .unwrap();
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn persistent_binding_survives_service_restart_and_repeated_login() {
    let mut f = fixture().await;
    operational(&f).await;
    let response = rpc(&f, "test.echo", json!({"text":"hello"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let expiry = f.portal.expiry.load(Ordering::SeqCst);
    f.app = application(&f.dir.path().join("service.db"), f.config.clone())
        .await
        .unwrap();
    login(&f).await;
    let (_database, conn) = db(&f).await;
    let row = conn
        .query("SELECT COUNT(*) FROM commercial_accounts", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 1);
    assert_eq!(f.portal.expiry.load(Ordering::SeqCst), expiry);
    assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn revocation_during_commercial_authorization_blocks_delivery_without_holding_store_lock() {
    let f = fixture().await;
    operational(&f).await;
    let cookie = login(&f).await;
    f.portal.gate_mode.store(2, Ordering::SeqCst);
    let app = f.app.clone();
    let pending = tokio::spawn(async move {
        request(&app,"POST","/mcp",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"test.echo","arguments":{"text":"hello"}}}),None,Some("access")).await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        f.portal.gate_started.notified(),
    )
    .await
    .unwrap();
    // An actual same-app mutation must complete while the provider HTTP reply is suspended.
    // This proves both the store mutex and its transaction are released across network I/O.
    let csrf = remote_service::security::csrf(cookie.strip_prefix("rdc_session=").unwrap());
    let revoke = f.app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/devices/device/revoke")
            .header("cookie", &cookie)
            .header("x-csrf-token", csrf)
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), revoke)
            .await
            .unwrap()
            .unwrap()
            .status(),
        StatusCode::OK
    );
    f.portal.gate_continue.notify_one();
    assert_eq!(pending.await.unwrap().status(), StatusCode::NOT_FOUND);
    let (_database, conn) = db(&f).await;
    let row = conn
        .query("SELECT COUNT(*) FROM commercial_admissions", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 0);
}
#[tokio::test]
async fn poll_rechecks_revocation_after_commercial_inspection() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    f.portal.gate_mode.store(1, Ordering::SeqCst);
    let app = f.app.clone();
    let pending = tokio::spawn(async move {
        request(
            &app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        f.portal.gate_started.notified(),
    )
    .await
    .unwrap();
    let (_database, conn) = db(&f).await;
    conn.execute("UPDATE grants SET revoked=1", ())
        .await
        .unwrap();
    f.portal.gate_continue.notify_one();
    assert_eq!(
        json_response(pending.await.unwrap()).await["commands"],
        json!([])
    );
    assert_eq!(
        f.portal.aborts.load(Ordering::SeqCst),
        1,
        "pre-dispatch revocation must not charge"
    );
}

#[tokio::test]
async fn portal_redirect_is_not_followed_or_given_a_secret() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sink_address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let sink = tokio::spawn(async move {
        let app = Router::new().route(
            "/stolen",
            post(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Json(json!({}))
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let portal_address = listener.local_addr().unwrap();
    let redirect = tokio::spawn(async move {
        let destination = format!("http://{sink_address}/stolen");
        let app = Router::new().route(
            "/api/mcp/exchange",
            post(move || {
                let destination = destination.clone();
                async move {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", destination)],
                        "",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(ORIGIN.into(), true, vec![]).unwrap();
    config.commercial = Some(
        CommercialConfig::new(format!("http://{portal_address}"), SECRET.into(), true).unwrap(),
    );
    let app = application(&dir.path().join("service.db"), config)
        .await
        .unwrap();
    let (cookie, state) = begin(&app).await;
    assert_eq!(
        callback(&app, &cookie, &state, "redirect-ticket")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    sink.abort();
    redirect.abort();
}

#[tokio::test]
async fn callback_without_bound_nonce_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::new("http://127.0.0.1:8787".into(), true, vec![]).unwrap();
    let app = application(&dir.path().join("service.db"), config)
        .await
        .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/commercial/callback?ticket=anything&state=anything")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn free_queue_inspects_poll_reserves_once_restart_preserves_deadline_and_result_completes() {
    let mut f = fixture().await;
    operational(&f).await;
    f.portal.free.store(1, Ordering::SeqCst);
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        f.portal.executions.load(Ordering::SeqCst),
        0,
        "queue must never reserve"
    );
    let first = json_response(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await,
    )
    .await;
    let command = &first["commands"][0];
    assert!(
        command.is_object(),
        "Free must dispatch without a paid entitlement: {first}"
    );
    let deadline = command["envelope"]["meta"]["deadline"].clone();
    let (_database, conn) = db(&f).await;
    conn.execute("UPDATE relay SET lease_until=0", ())
        .await
        .unwrap();
    f.portal.lease_ms.store(1000, Ordering::SeqCst);
    f.app = application(&f.dir.path().join("service.db"), f.config.clone())
        .await
        .unwrap();
    let second = json_response(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await,
    )
    .await;
    assert_eq!(
        second["commands"][0]["envelope"]["meta"]["deadline"],
        deadline
    );
    assert_eq!(
        f.portal.executions.load(Ordering::SeqCst),
        1,
        "restart must not reserve again"
    );
    let body = json!({"request_id":command["request_id"],"envelope":{"meta":command["envelope"]["meta"],"payload":{"Ok":{"done":true}}}});
    let mut envelope: runtime_protocol::RequestEnvelope =
        serde_json::from_value(command["envelope"].clone()).unwrap();
    envelope.meta.deadline = deadline.as_u64().unwrap();
    let response = runtime_protocol::ResponseEnvelope {
        meta: envelope.meta,
        payload: Ok(json!({"status":"Succeeded","done":true})),
    };
    let body = json!({"request_id":body["request_id"],"envelope":response});
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/agent/result",
            body,
            None,
            Some("agent")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(f.portal.completions.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn exhausted_free_returns_reset_and_upgrade_without_dispatch() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    f.portal.expiry.store(-1, Ordering::SeqCst);
    let response = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let body = json_response(response).await;
    assert_eq!(body["error"], "FREE_DAILY_LIMIT_REACHED");
    assert!(body["resets_at"].as_i64().unwrap() > now());
    assert!(body["upgrade_url"].as_str().unwrap().contains("dashboard"));
}

#[tokio::test]
async fn free_rejects_persistent_process_and_terminal_before_dispatch() {
    for name in ["terminal.open", "terminal.write", "process.spawn"] {
        let f = fixture().await;
        operational(&f).await;
        f.portal.free.store(1, Ordering::SeqCst);
        let (_database, conn) = db(&f).await;
        let tools = json!([{"name":name,"version":"1.0.0","description":"Persistent fixture","capabilities":[],"risk":"high","input_schema":{"type":"object"},"output_schema":null}]);
        conn.execute("UPDATE devices SET tools=?1", params![tools.to_string()])
            .await
            .unwrap();
        let response = rpc(&f, name, json!({})).await;
        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            json_response(response).await["error"],
            "FREE_OPERATION_NOT_BOUNDED"
        );
        assert_eq!(f.portal.executions.load(Ordering::SeqCst), 0);
        let count = conn
            .query("SELECT COUNT(*) FROM relay", ())
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap()
            .get::<i64>(0)
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn exhausted_normal_queue_does_not_starve_owned_recovery() {
    let f = fixture().await;
    let owner = operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    let (_database, conn) = db(&f).await;
    let meta = runtime_protocol::Metadata::new(Some(uuid::Uuid::new_v4()), 120000);
    let envelope = runtime_protocol::RequestEnvelope {
        meta: meta.clone(),
        payload: runtime_protocol::Request::Execute {
            tool: "operation.cancel".into(),
            input: json!({"operation_id":uuid::Uuid::new_v4()}),
        },
    };
    conn.execute("INSERT INTO relay(id,device_id,user_id,grant_family,operation_id,dedupe_key,envelope,created_at,deadline) VALUES(?1,'device',?2,'family',?3,?3,?4,?5,?6)",params![meta.request_id.to_string(),owner,meta.operation_id.to_string(),serde_json::to_string(&envelope).unwrap(),now()+1,meta.deadline as i64]).await.unwrap();
    conn.execute(
        "INSERT INTO commercial_admissions VALUES(?1,1)",
        params![meta.request_id.to_string()],
    )
    .await
    .unwrap();
    f.portal.expiry.store(-1, Ordering::SeqCst);
    let response = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_response(response).await;
    assert_eq!(body["commands"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["commands"][0]["request_id"],
        meta.request_id.to_string()
    );
}

#[tokio::test]
async fn disconnected_agent_response_does_not_release_reserved_usage() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    let body = json_response(
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await,
    )
    .await;
    let command = &body["commands"][0];
    let request: runtime_protocol::RequestEnvelope =
        serde_json::from_value(command["envelope"].clone()).unwrap();
    let response = runtime_protocol::ResponseEnvelope {
        meta: request.meta,
        payload: Err(runtime_protocol::ErrorEnvelope::new(
            runtime_protocol::ErrorCode::Disconnected,
            "IPC lost",
        )),
    };
    assert_eq!(
        request_result(&f, command["request_id"].clone(), response).await,
        StatusCode::OK
    );
    assert_eq!(
        f.portal.completions.load(Ordering::SeqCst),
        0,
        "IPC loss cannot prove detached daemon ended"
    );
}
async fn request_result(
    f: &Fixture,
    id: Value,
    envelope: runtime_protocol::ResponseEnvelope,
) -> StatusCode {
    request(
        &f.app,
        "POST",
        "/api/agent/result",
        json!({"request_id":id,"envelope":envelope}),
        None,
        Some("agent"),
    )
    .await
    .status()
}

#[tokio::test]
async fn owned_recovery_has_reserved_queue_capacity_after_free_exhaustion() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    let (_database, conn) = db(&f).await;
    let row = conn
        .query("SELECT operation_id FROM relay", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    let operation: String = row.get(0).unwrap();
    conn.execute("UPDATE relay SET lease_until=?1", params![now() + 30])
        .await
        .unwrap();
    let row = conn
        .query("SELECT tools FROM devices", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    let mut tools: Vec<Value> = serde_json::from_str(&row.get::<String>(0).unwrap()).unwrap();
    tools.push(json!({"name":"operation.cancel","version":"1.0.0","description":"Cancel","capabilities":[],"risk":"write","input_schema":{"type":"object"},"output_schema":null}));
    conn.execute(
        "UPDATE devices SET tools=?1",
        params![serde_json::to_string(&tools).unwrap()],
    )
    .await
    .unwrap();
    f.portal.expiry.store(-1, Ordering::SeqCst);
    let calls = f.portal.usage_calls.lock().unwrap().len();
    assert_eq!(
        rpc(&f, "operation.cancel", json!({"operation_id":operation}))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(f.portal.usage_calls.lock().unwrap().len(), calls);
    assert_eq!(
        rpc(
            &f,
            "operation.cancel",
            json!({"operation_id":uuid::Uuid::new_v4()})
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn concurrent_poll_does_not_abort_or_reserve_another_dispatch_claim() {
    let f = fixture().await;
    operational(&f).await;
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    f.portal.gate_mode.store(1, Ordering::SeqCst);
    let app = f.app.clone();
    let first = tokio::spawn(async move {
        request(
            &app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        f.portal.gate_started.notified(),
    )
    .await
    .unwrap();
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        request(
            &f.app,
            "GET",
            "/api/agent/poll",
            Value::Null,
            None,
            Some("agent"),
        ),
    )
    .await
    .expect("concurrent poll attempted same reservation HTTP");
    assert_eq!(json_response(second).await["commands"], json!([]));
    f.portal.gate_continue.notify_one();
    assert_eq!(
        json_response(first.await.unwrap()).await["commands"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(f.portal.aborts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sql_relative_budget_bounds_dispatch_even_when_reported_epoch_is_behind_service() {
    let f = fixture().await;
    operational(&f).await;
    f.portal.lease_ms.store(1000, Ordering::SeqCst);
    f.portal.expiry_shift.store(-600, Ordering::SeqCst);
    assert_eq!(
        rpc(&f, "test.echo", json!({"text":"hello"})).await.status(),
        StatusCode::OK
    );
    let response = request(
        &f.app,
        "GET",
        "/api/agent/poll",
        Value::Null,
        None,
        Some("agent"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_response(response).await;
    let command = &body["commands"][0];
    assert!(command.is_object(), "{body}");
    assert!(command["server_budget_ms"].as_u64().unwrap() <= 1000);
    assert!(
        command["envelope"]["meta"]["deadline"].as_u64().unwrap()
            <= runtime_protocol::now_ms() + 1000
    );
}

#[tokio::test]
async fn recovery_handles_require_matching_successful_producer_and_remain_after_history_pruning() {
    for (producer, control, field) in [
        ("process.spawn", "process.kill", "process_id"),
        ("terminal.open", "terminal.close", "terminal_id"),
    ] {
        let f = fixture().await;
        operational(&f).await;
        let (_database, conn) = db(&f).await;
        let tools=vec![producer,control,"test.echo"].into_iter().map(|name|json!({"name":name,"version":"1.0.0","description":"Fixture","capabilities":[],"risk":"write","input_schema":{"type":"object"},"output_schema":null})).collect::<Vec<_>>();
        conn.execute(
            "UPDATE devices SET tools=?1",
            params![serde_json::to_string(&tools).unwrap()],
        )
        .await
        .unwrap();
        assert_eq!(
            rpc(&f, "test.echo", json!({})).await.status(),
            StatusCode::OK
        );
        let command = json_response(
            request(
                &f.app,
                "GET",
                "/api/agent/poll",
                Value::Null,
                None,
                Some("agent"),
            )
            .await,
        )
        .await["commands"][0]
            .clone();
        let original_request: runtime_protocol::RequestEnvelope =
            serde_json::from_value(command["envelope"].clone()).unwrap();
        let resource = uuid::Uuid::new_v4();
        let mut output = json!({"status":"Succeeded","tool":"test.echo","result":{}});
        output["result"][field] = json!(resource);
        let reply = runtime_protocol::ResponseEnvelope {
            meta: original_request.meta,
            payload: Ok(output),
        };
        assert_eq!(
            request_result(&f, command["request_id"].clone(), reply).await,
            StatusCode::OK
        );
        f.portal.expiry.store(-1, Ordering::SeqCst);
        let mut args = json!({});
        args[field] = json!(resource);
        assert_eq!(
            rpc(&f, control, args.clone()).await.status(),
            StatusCode::FORBIDDEN,
            "arbitrary read output must not establish ownership"
        );
        f.portal.expiry.store(0, Ordering::SeqCst);
        assert_eq!(rpc(&f, producer, json!({})).await.status(), StatusCode::OK);
        let produced = json_response(
            request(
                &f.app,
                "GET",
                "/api/agent/poll",
                Value::Null,
                None,
                Some("agent"),
            )
            .await,
        )
        .await["commands"][0]
            .clone();
        let produced_request: runtime_protocol::RequestEnvelope =
            serde_json::from_value(produced["envelope"].clone()).unwrap();
        let mut output = json!({"status":"Succeeded","tool":producer,"result":{}});
        output["result"][field] = json!(resource);
        assert_eq!(
            request_result(
                &f,
                produced["request_id"].clone(),
                runtime_protocol::ResponseEnvelope {
                    meta: produced_request.meta,
                    payload: Ok(output)
                }
            )
            .await,
            StatusCode::OK
        );
        f.portal.expiry.store(-1, Ordering::SeqCst);
        conn.execute("DELETE FROM relay", ()).await.unwrap();
        assert_eq!(
            rpc(&f, control, args).await.status(),
            StatusCode::OK,
            "owned handle recovery survives request retention"
        );
        let mut other = json!({});
        other[field] = json!(uuid::Uuid::new_v4());
        assert_eq!(
            rpc(&f, control, other).await.status(),
            StatusCode::FORBIDDEN
        );
    }
}
