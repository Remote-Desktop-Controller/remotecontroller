use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use remote_service::{Config, application};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: Option<&str>,
    csrf: Option<&str>,
    bearer: Option<&str>,
) -> (StatusCode, Value, Option<String>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(v) = cookie {
        request = request.header("cookie", v);
    }
    if let Some(v) = csrf {
        request = request.header("x-csrf-token", v);
    }
    if let Some(v) = bearer {
        request = request.header("authorization", format!("Bearer {v}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}
async fn setup() -> (tempfile::TempDir, Router) {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("remote_service=debug")
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(
        "http://127.0.0.1:8787".into(),
        true,
        vec!["https://client.example/callback".into()],
    )
    .unwrap();
    config.poll_seconds = 0;
    config.call_seconds = 1;
    config.queue_limit = 2;
    let app = application(&dir.path().join("service.db"), config)
        .await
        .unwrap();
    (dir, app)
}
async fn form(
    app: &Router,
    path: &str,
    fields: &[(&str, &str)],
    cookie: Option<&str>,
) -> (StatusCode, Value, Option<String>) {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(encoded)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        location,
    )
}
async fn authorization(app: &Router, cookie: &str, csrf: &str) -> (String, String, String) {
    let (status, client, _) = send(
        app,
        "POST",
        "/oauth/register",
        json!({"client_name":"Test MCP","redirect_uris":["https://client.example/callback"]}),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = client["client_id"].as_str().unwrap().to_owned();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned();
    let challenge = remote_service::security::hash(&verifier);
    let fields = vec![
        ("client_id", client_id.as_str()),
        ("redirect_uri", "https://client.example/callback"),
        ("response_type", "code"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", "http://127.0.0.1:8787/mcp"),
        ("scope", "devices:read tools:read tools:execute"),
        ("state", "test-state"),
        ("csrf", csrf),
        ("decision", "allow"),
    ];
    let (status, data, location) = form(app, "/oauth/authorize", &fields, Some(cookie)).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{}", data);
    let url = url::Url::parse(location.as_deref().unwrap()).unwrap();
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .to_string();
    assert_eq!(
        url.query_pairs().find(|(key, _)| key == "state").unwrap().1,
        "test-state"
    );
    (client_id, code, verifier)
}
async fn oauth_tokens(app: &Router, cookie: &str, csrf: &str) -> (String, Value) {
    let (client, code, verifier) = authorization(app, cookie, csrf).await;
    let (status, tokens, _) = form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", "https://client.example/callback"),
            ("resource", "http://127.0.0.1:8787/mcp"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", tokens);
    (client, tokens)
}
async fn account(app: &Router, email: &str) -> (String, String) {
    let (status, data, cookie) = send(
        app,
        "POST",
        "/api/auth/register",
        json!({"email":email,"password":"LongPasswordForTest123!"}),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", data);
    (
        cookie.unwrap(),
        data["csrf_token"].as_str().unwrap().to_owned(),
    )
}
async fn pair(app: &Router, cookie: &str, csrf: &str, name: &str) -> Value {
    let (status, pairing, _) = send(
        app,
        "POST",
        "/api/pairings",
        json!({"name":name,"workspace_id":uuid::Uuid::new_v4().to_string(),"tools":[]}),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = send(
        app,
        "POST",
        "/api/pairings/approve",
        json!({"code":pairing["pairing_code"]}),
        Some(cookie),
        Some(csrf),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, approved, _) = send(
        app,
        "POST",
        &format!(
            "/api/pairings/{}/poll",
            pairing["pairing_id"].as_str().unwrap()
        ),
        json!({"poll_token":pairing["poll_token"]}),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");
    approved
}
#[tokio::test]
async fn two_accounts_are_isolated_and_revoke_invalidates_agent() {
    let (_dir, app) = setup().await;
    let (alice, alice_csrf) = account(&app, "alice@example.test").await;
    let (bob, bob_csrf) = account(&app, "bob@example.test").await;
    let device = pair(&app, &alice, &alice_csrf, "Notebook").await;
    let (_, list, _) = send(
        &app,
        "GET",
        "/api/devices",
        json!({}),
        Some(&bob),
        None,
        None,
    )
    .await;
    assert_eq!(list["devices"].as_array().unwrap().len(), 0);
    let revoke = format!(
        "/api/devices/{}/revoke",
        device["device_id"].as_str().unwrap()
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &revoke,
            json!({}),
            Some(&bob),
            Some(&bob_csrf),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, "POST", &revoke, json!({}), Some(&alice), None, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &revoke,
            json!({}),
            Some(&alice),
            Some(&alice_csrf),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/agent/poll",
            json!({}),
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn pairing_poll_secrets_and_approval_are_single_use() {
    let (_dir, app) = setup().await;
    let (cookie, csrf) = account(&app, "owner@example.test").await;
    let (_, pairing, _) = send(
        &app,
        "POST",
        "/api/pairings",
        json!({"name":"PC","workspace_id":uuid::Uuid::new_v4().to_string(),"tools":[]}),
        None,
        None,
        None,
    )
    .await;
    let path = format!(
        "/api/pairings/{}/poll",
        pairing["pairing_id"].as_str().unwrap()
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &path,
            json!({"poll_token":"wrong"}),
            None,
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/pairings/approve",
            json!({"code":pairing["pairing_code"]}),
            Some(&cookie),
            Some(&csrf),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    // Duplicate approval by the SAME owner is idempotent (returns OK, no conflict / already_used)
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/pairings/approve",
            json!({"code":pairing["pairing_code"]}),
            Some(&cookie),
            Some(&csrf),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    // But approval by another user on an already claimed pairing is rejected with CONFLICT
    let (other_cookie, other_csrf) = account(&app, "other-user@example.test").await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/pairings/approve",
            json!({"code":pairing["pairing_code"]}),
            Some(&other_cookie),
            Some(&other_csrf),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, first, _) = send(
        &app,
        "POST",
        &path,
        json!({"poll_token":pairing["poll_token"]}),
        None,
        None,
        None,
    )
    .await;
    assert!(first["device_token"].is_string());
    let (_, second, _) = send(
        &app,
        "POST",
        &path,
        json!({"poll_token":pairing["poll_token"]}),
        None,
        None,
        None,
    )
    .await;
    assert!(second["device_token"].is_null());
}
#[tokio::test]
async fn cors_origin_and_dynamic_redirect_policy_reject_attackers() {
    let (_dir, app) = setup().await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("origin", "https://evil.example")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"email":"victim@example.test","password":"LongPasswordForTest123!"}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/oauth/register",
            json!({"client_name":"evil","redirect_uris":["https://evil.example/callback"]}),
            None,
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn account_form_errors_are_portuguese_html_and_connection_details_are_collapsed() {
    let (_dir, app) = setup().await;
    for path in ["/login", "/register"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "email=owner%40example.test&password=LongPasswordForTest123%21&csrf=wrong",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let html = String::from_utf8(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("Este formulário expirou"));
        assert!(html.contains("Voltar e tentar novamente"));
        assert!(!html.contains("invalid_request"));
    }
    let (cookie, csrf) = account(&app, "ui@example.test").await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pair")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("code=INVALID&csrf=wrong"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("Este formulário expirou"));
    assert!(html.contains("Voltar e tentar novamente"));

    // Verify /pair form submission redirects to /?paired=1
    let (_, pair_item, _) = send(
        &app,
        "POST",
        "/api/pairings",
        json!({"name":"UI-PC","workspace_id":uuid::Uuid::new_v4().to_string(),"tools":[]}),
        None,
        None,
        None,
    )
    .await;
    let code = pair_item["pairing_code"].as_str().unwrap();
    let pair_form_body = format!("code={code}&csrf={csrf}");
    let res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pair")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(pair_form_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::SEE_OTHER);
    assert_eq!(res1.headers()["location"], "/?paired=1");

    // Duplicate submit by same user is idempotent and redirects to /?paired=1
    let res2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pair")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(pair_form_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::SEE_OTHER);
    assert_eq!(res2.headers()["location"], "/?paired=1");

    // GET /?paired=1 renders the success notice banner
    let res_home = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/?paired=1")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_home.status(), StatusCode::OK);
    let home_html = String::from_utf8(
        to_bytes(res_home.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(home_html.contains("Computador pareado com sucesso!"));
    assert!(home_html.contains("<details>"));
    assert!(home_html.contains("Conexão de teste"));
    assert!(!home_html.contains("device.list"));
    assert!(!home_html.contains("daemon"));
}

#[tokio::test]
async fn oauth_pkce_wrong_resource_code_replay_and_refresh_replay_are_rejected() {
    let (_dir, app) = setup().await;
    let (cookie, csrf) = account(&app, "oauth@example.test").await;
    let (client, code, verifier) = authorization(&app, &cookie, &csrf).await;
    let mut fields = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client.as_str()),
        ("code", code.as_str()),
        ("code_verifier", "wrong-verifier"),
        ("redirect_uri", "https://client.example/callback"),
        ("resource", "http://127.0.0.1:8787/mcp"),
    ];
    assert_eq!(
        form(&app, "/oauth/token", &fields, None).await.0,
        StatusCode::BAD_REQUEST
    );
    fields[3].1 = &verifier;
    fields[5].1 = "https://other.example/mcp";
    assert_eq!(
        form(&app, "/oauth/token", &fields, None).await.0,
        StatusCode::BAD_REQUEST
    );
    fields[5].1 = "http://127.0.0.1:8787/mcp";
    fields[4].1 = "https://client.example/callback/";
    assert_eq!(
        form(&app, "/oauth/token", &fields, None).await.0,
        StatusCode::BAD_REQUEST
    );
    fields[4].1 = "https://client.example/callback";
    let (status, tokens, _) = form(&app, "/oauth/token", &fields, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        form(&app, "/oauth/token", &fields, None).await.0,
        StatusCode::BAD_REQUEST
    );
    let refresh = vec![
        ("grant_type", "refresh_token"),
        ("client_id", client.as_str()),
        ("refresh_token", tokens["refresh_token"].as_str().unwrap()),
        ("resource", "http://127.0.0.1:8787/mcp"),
    ];
    let (status, rotated, _) = form(&app, "/oauth/token", &refresh, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(tokens["refresh_token"], rotated["refresh_token"]);
    assert_eq!(
        form(&app, "/oauth/token", &refresh, None).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/mcp",
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
            None,
            None,
            rotated["access_token"].as_str()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn expired_pairing_is_never_delivered() {
    let (dir, app) = setup().await;
    let (_, pairing, _) = send(
        &app,
        "POST",
        "/api/pairings",
        json!({"name":"old","workspace_id":uuid::Uuid::new_v4().to_string(),"tools":[]}),
        None,
        None,
        None,
    )
    .await;
    let database = libsql::Builder::new_local(dir.path().join("service.db"))
        .build()
        .await
        .unwrap();
    database
        .connect()
        .unwrap()
        .execute("UPDATE pairings SET expires_at=0", ())
        .await
        .unwrap();
    let (_, expired, _) = send(
        &app,
        "POST",
        &format!(
            "/api/pairings/{}/poll",
            pairing["pairing_id"].as_str().unwrap()
        ),
        json!({"poll_token":pairing["poll_token"]}),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(expired["status"], "expired");
    assert!(expired["device_token"].is_null());
}
#[tokio::test]
async fn relay_preserves_identity_checks_results_and_bounds_queue() {
    let (dir, mut app) = setup().await;
    let (cookie, csrf) = account(&app, "relay@example.test").await;
    let device = pair(&app, &cookie, &csrf, "Remote PC").await;
    let database = libsql::Builder::new_local(dir.path().join("service.db"))
        .build()
        .await
        .unwrap();
    let connection = database.connect().unwrap();
    let mut rows = connection
        .query(
            "SELECT workspace_id FROM devices WHERE id=?1",
            [device["device_id"].as_str().unwrap()],
        )
        .await
        .unwrap();
    let workspace: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
    drop(rows);
    let descriptor = json!({"name":"file.read","version":"1","description":"Read","capabilities":["read"],"risk":"read","input_schema":{"type":"object"},"output_schema":{"type":"object"}});
    let session_descriptor = json!({"name":"session.resume","version":"1","description":"Resume","capabilities":["write"],"risk":"write","input_schema":{"type":"object","properties":{"session_id":{"type":"string","format":"uuid"}},"required":["session_id"],"additionalProperties":false},"output_schema":{"type":"object"}});
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/heartbeat",
            json!({"workspace_id":workspace,"tools":[descriptor,session_descriptor]}),
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );
    let (client, tokens) = oauth_tokens(&app, &cookie, &csrf).await;
    let token = tokens["access_token"].as_str().unwrap();
    let (_, catalog, _) = send(
        &app,
        "POST",
        "/mcp",
        json!({"jsonrpc":"2.0","id":70,"method":"tools/list"}),
        None,
        None,
        Some(token),
    )
    .await;
    assert!(
        catalog["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "file.read")
    );
    let operation = uuid::Uuid::new_v4();
    let session = uuid::Uuid::new_v4();
    let request = json!({"jsonrpc":"2.0","id":77,"method":"tools/call","params":{"name":"file.read","arguments":{"device_id":device["device_id"],"session_id":session,"path":"a.txt"},"_meta":{"runtimeOperationId":operation}}});
    let (status, pending, _) = send(
        &app,
        "POST",
        "/mcp",
        request.clone(),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", pending);
    assert_eq!(pending["result"]["structuredContent"]["status"], "pending");
    let mut restarted_config = Config::new(
        "http://127.0.0.1:8787".into(),
        true,
        vec!["https://client.example/callback".into()],
    )
    .unwrap();
    restarted_config.poll_seconds = 0;
    restarted_config.call_seconds = 1;
    restarted_config.queue_limit = 2;
    app = application(&dir.path().join("service.db"), restarted_config)
        .await
        .unwrap();
    let (_, commands, _) = send(
        &app,
        "GET",
        "/api/agent/poll",
        json!({}),
        None,
        None,
        device["device_token"].as_str(),
    )
    .await;
    let command = &commands["commands"][0];
    let (bob_cookie, bob_csrf) = account(&app, "other-relay@example.test").await;
    let (_, bob_tokens) = oauth_tokens(&app, &bob_cookie, &bob_csrf).await;
    assert_eq!(
        send(
            &app,
            "POST",
            "/mcp",
            request.clone(),
            None,
            None,
            bob_tokens["access_token"].as_str()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let bob_device = pair(&app, &bob_cookie, &bob_csrf, "Other PC").await;
    assert_eq!(
        command["envelope"]["meta"]["operation_id"],
        operation.to_string()
    );
    assert_eq!(
        command["envelope"]["meta"]["session_id"],
        session.to_string()
    );
    assert!(command["envelope"]["payload"]["Execute"]["input"]["device_id"].is_null());
    let mut response = json!({"request_id":command["request_id"],"envelope":{"meta":command["envelope"]["meta"],"payload":{"Ok":{"text":"hello"}}}});
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/result",
            response.clone(),
            None,
            None,
            bob_device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    response["envelope"]["meta"]["operation_id"] = json!(uuid::Uuid::new_v4());
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/result",
            response.clone(),
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    response["envelope"]["meta"] = command["envelope"]["meta"].clone();
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/result",
            response.clone(),
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/result",
            response,
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, finished, _) = send(
        &app,
        "POST",
        "/mcp",
        request.clone(),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(
        finished["result"]["structuredContent"]["result"]["text"],
        "hello"
    );
    let mut argument_retry = request.clone();
    argument_retry["id"] = json!(177);
    argument_retry["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("runtimeOperationId");
    argument_retry["params"]["arguments"]["runtime_operation_id"] = json!(operation);
    let (status, retried, _) = send(
        &app,
        "POST",
        "/mcp",
        argument_retry,
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", retried);
    assert_eq!(
        retried["result"]["structuredContent"]["request_id"],
        finished["result"]["structuredContent"]["request_id"]
    );
    let mut conflicting_identity = request.clone();
    conflicting_identity["params"]["arguments"]["runtime_operation_id"] =
        json!(uuid::Uuid::new_v4());
    assert_eq!(
        send(
            &app,
            "POST",
            "/mcp",
            conflicting_identity,
            None,
            None,
            Some(token)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let local_session = uuid::Uuid::new_v4();
    let resume = json!({"jsonrpc":"2.0","id":178,"method":"tools/call","params":{"name":"session.resume","arguments":{"device_id":device["device_id"],"session_id":local_session}}});
    assert_eq!(
        send(&app, "POST", "/mcp", resume, None, None, Some(token))
            .await
            .0,
        StatusCode::OK
    );
    let (_, resume_commands, _) = send(
        &app,
        "GET",
        "/api/agent/poll",
        json!({}),
        None,
        None,
        device["device_token"].as_str(),
    )
    .await;
    let resumed = &resume_commands["commands"][0];
    assert_eq!(
        resumed["envelope"]["payload"]["Execute"]["input"]["session_id"],
        local_session.to_string()
    );
    assert!(resumed["envelope"]["meta"]["session_id"].is_null());
    let resume_response = json!({"request_id":resumed["request_id"],"envelope":{"meta":resumed["envelope"]["meta"],"payload":{"Ok":{"resumed":true}}}});
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/result",
            resume_response,
            None,
            None,
            device["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut changed = request.clone();
    changed["params"]["arguments"]["path"] = json!("different.txt");
    assert_eq!(
        send(&app, "POST", "/mcp", changed, None, None, Some(token))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut new_operations = Vec::new();
    for id in [78, 78] {
        let mut queued = request.clone();
        queued["id"] = json!(id);
        queued["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove("runtimeOperationId");
        let (status, queued, _) = send(&app, "POST", "/mcp", queued, None, None, Some(token)).await;
        assert_eq!(status, StatusCode::OK, "{}", queued);
        new_operations.push(queued["result"]["structuredContent"]["operation_id"].clone());
    }
    assert_ne!(
        new_operations[0], new_operations[1],
        "Reusing RPC id in a new chat must not suppress an intentional operation"
    );
    let mut over = request;
    over["id"] = json!(80);
    over["params"]["_meta"]["runtimeOperationId"] = json!(uuid::Uuid::new_v4());
    assert_eq!(
        send(&app, "POST", "/mcp", over, None, None, Some(token))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        form(
            &app,
            "/oauth/revoke",
            &[("client_id", client.as_str()), ("token", token)],
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, after_revoke, _) = send(
        &app,
        "GET",
        "/api/agent/poll",
        json!({}),
        None,
        None,
        device["device_token"].as_str(),
    )
    .await;
    assert_eq!(after_revoke["commands"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn browser_html_forms_register_login_and_logout_with_real_cookies() {
    let (_dir, app) = setup().await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.headers()["referrer-policy"], "strict-origin");
    let login_cookie = page.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = login_cookie.strip_prefix("rdc_login_csrf=").unwrap();
    let html =
        String::from_utf8(to_bytes(page.into_body(), 65536).await.unwrap().to_vec()).unwrap();
    assert!(html.contains(&format!("name=csrf value=\"{csrf}\"")));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("email", "forms@example.test"),
            ("password", "Forms-test-password-2026!"),
            ("csrf", csrf),
            ("next", "/"),
        ])
        .finish();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("origin", "http://127.0.0.1:8787")
                .header("cookie", &login_cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/");
    let session_cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let (status, session, _) = send(
        &app,
        "GET",
        "/api/session",
        json!(null),
        Some(&session_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["email"], "forms@example.test");
    assert_eq!(
        form(
            &app,
            "/logout",
            &[("csrf", session["csrf_token"].as_str().unwrap())],
            Some(&session_cookie)
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/session",
            json!(null),
            Some(&session_cookie),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("origin", "http://127.0.0.1:8787")
                .header("cookie", &login_cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("rdc_session=")
    );
}

async fn issuer_app(redirects: Vec<String>) -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::new("https://issuer.example.test:9443/".into(), false, redirects).unwrap();
    let app = application(&dir.path().join("service.db"), config)
        .await
        .unwrap();
    (dir, app)
}
const CHATGPT_CALLBACK: &str = "https://chatgpt.com/connector_platform_oauth_redirect";

#[tokio::test]
async fn issuer_metadata_identifies_the_same_server_for_stable_chatgpt_callback() {
    let (_dir, app) = issuer_app(vec![CHATGPT_CALLBACK.into()]).await;
    let (status, metadata, _) = send(
        &app,
        "GET",
        "/.well-known/oauth-authorization-server",
        json!(null),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(metadata["issuer"], "https://issuer.example.test:9443");
    assert_eq!(
        metadata["authorization_response_iss_parameter_supported"],
        true
    );
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(metadata["client_id_metadata_document_supported"], false);
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let (status, resource, _) = send(&app, "GET", path, json!(null), None, None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            resource["authorization_servers"],
            json!(["https://issuer.example.test:9443"])
        );
        assert_eq!(resource["resource"], "https://issuer.example.test:9443/mcp");
    }
}

#[tokio::test]
async fn stable_callback_dcr_still_requires_explicit_exact_allowlisting() {
    let (_dir, app) = issuer_app(vec![CHATGPT_CALLBACK.into()]).await;
    for (redirect, expected) in [
        (CHATGPT_CALLBACK, StatusCode::CREATED),
        (
            "https://chatgpt.com/connector_platform_oauth_redirect/",
            StatusCode::BAD_REQUEST,
        ),
        (
            "https://chatgpt.com/connector_platform_oauth_redirect?changed=1",
            StatusCode::BAD_REQUEST,
        ),
        (
            "https://chatgpt.com/connector/oauth/other-connection",
            StatusCode::BAD_REQUEST,
        ),
        (
            "https://chatgpt.com.evil.example/connector_platform_oauth_redirect",
            StatusCode::BAD_REQUEST,
        ),
        ("https://chatgpt.com/*", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            send(
                &app,
                "POST",
                "/oauth/register",
                json!({"redirect_uris":[redirect]}),
                None,
                None,
                None
            )
            .await
            .0,
            expected,
            "{redirect}"
        );
    }
    let (_empty_dir, empty) = issuer_app(vec![]).await;
    assert_eq!(
        send(
            &empty,
            "POST",
            "/oauth/register",
            json!({"redirect_uris":[CHATGPT_CALLBACK]}),
            None,
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

async fn check_authorization_issuer(decision: &str) {
    let custom = "https://client.example/callback?tracking=a%2Bb";
    let (_dir, app) = issuer_app(vec![CHATGPT_CALLBACK.into(), custom.into()]).await;
    let (cookie, csrf) = account(&app, "issuer@example.test").await;
    for callback in [CHATGPT_CALLBACK, custom] {
        let (status, registered, _) = send(
            &app,
            "POST",
            "/oauth/register",
            json!({"redirect_uris":[callback]}),
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let client = registered["client_id"].as_str().unwrap();
        let mut fields = vec![
            ("client_id", client),
            ("redirect_uri", callback),
            ("response_type", "code"),
            (
                "code_challenge",
                "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            ),
            ("code_challenge_method", "S256"),
            ("resource", "https://issuer.example.test:9443/mcp"),
            ("state", "state with + & / unicode ç"),
            ("csrf", csrf.as_str()),
            ("decision", decision),
        ];
        for (index, invalid) in [
            (4, "plain"),
            (4, ""),
            (5, "https://other.example/mcp"),
            (1, "https://evil.example/callback"),
        ] {
            let previous = fields[index].1;
            fields[index].1 = invalid;
            let (status, _, location) =
                form(&app, "/oauth/authorize", &fields, Some(&cookie)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(location.is_none(), "Invalid request redirected to a client");
            fields[index].1 = previous;
        }
        let (status, _, location) = form(&app, "/oauth/authorize", &fields, Some(&cookie)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let redirect = url::Url::parse(location.as_deref().unwrap()).unwrap();
        let pairs: Vec<_> = redirect.query_pairs().collect();
        let issuers: Vec<_> = pairs
            .iter()
            .filter(|(key, _)| key == "iss")
            .map(|(_, value)| value.as_ref())
            .collect();
        assert_eq!(issuers, vec!["https://issuer.example.test:9443"]);
        assert_eq!(
            pairs.iter().find(|(key, _)| key == "state").unwrap().1,
            "state with + & / unicode ç"
        );
        if callback == custom {
            assert_eq!(
                pairs.iter().find(|(key, _)| key == "tracking").unwrap().1,
                "a+b"
            );
        }
        if decision == "allow" {
            assert!(
                pairs
                    .iter()
                    .any(|(key, value)| key == "code" && !value.is_empty())
            );
            assert!(!pairs.iter().any(|(key, _)| key == "error"));
        } else {
            assert_eq!(
                pairs.iter().find(|(key, _)| key == "error").unwrap().1,
                "access_denied"
            );
            assert!(!pairs.iter().any(|(key, _)| key == "code"));
        }
    }
}
#[tokio::test]
async fn authorization_success_includes_exact_issuer() {
    check_authorization_issuer("allow").await;
}
#[tokio::test]
async fn authorization_access_denied_includes_exact_issuer() {
    check_authorization_issuer("deny").await;
}

#[tokio::test]
async fn mcp_exposes_tools_with_multiple_devices_and_supports_underscore_aliases() {
    let (dir, app) = setup().await;
    let (cookie, csrf) = account(&app, "multi-device@example.test").await;

    // Device 1: old device with no tools or stale heartbeat
    let _device1 = pair(&app, &cookie, &csrf, "Old PC").await;

    // Device 2: current active device with workspace tools
    let device2 = pair(&app, &cookie, &csrf, "Active PC").await;
    let database = libsql::Builder::new_local(dir.path().join("service.db"))
        .build()
        .await
        .unwrap();
    let connection = database.connect().unwrap();
    let mut rows = connection
        .query(
            "SELECT workspace_id FROM devices WHERE id=?1",
            [device2["device_id"].as_str().unwrap()],
        )
        .await
        .unwrap();
    let workspace2: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
    drop(rows);

    let descriptor = json!({
        "name": "filesystem.read",
        "version": "1",
        "description": "Read file content",
        "capabilities": ["read"],
        "risk": "read",
        "input_schema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        },
        "output_schema": {"type": "object"}
    });
    let list_descriptor = json!({
        "name": "filesystem.list",
        "version": "1",
        "description": "List directory",
        "capabilities": ["read"],
        "risk": "read",
        "input_schema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": []
        },
        "output_schema": {"type": "object"}
    });

    // Device 2 sends heartbeat with the active tools
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/agent/heartbeat",
            json!({"workspace_id": workspace2, "tools": [descriptor, list_descriptor]}),
            None,
            None,
            device2["device_token"].as_str()
        )
        .await
        .0,
        StatusCode::OK
    );

    let (_client, tokens) = oauth_tokens(&app, &cookie, &csrf).await;
    let token = tokens["access_token"].as_str().unwrap();

    // 1. Initial tools/list call from MCP client
    let (status, catalog, _) = send(
        &app,
        "POST",
        "/mcp",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", catalog);
    let tools = catalog["result"]["tools"].as_array().expect("tools array");

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    println!("MCP Tools exposed: {:?}", tool_names);
    assert!(
        tool_names.iter().any(|&n| n == "filesystem.read" || n == "filesystem_read"),
        "filesystem.read missing from tools list: {:?}",
        tool_names
    );

    // 2. Test tool execution with underscore aliases:
    // a) device_list
    let (status, res_list, _) = send(
        &app,
        "POST",
        "/mcp",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "device_list", "arguments": {}}
        }),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", res_list);
    assert!(res_list["error"].is_null(), "device_list failed: {:?}", res_list);

    // b) device_select
    let (status, res_select, _) = send(
        &app,
        "POST",
        "/mcp",
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "device_select",
                "arguments": {"device_id": device2["device_id"]}
            }
        }),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", res_select);
    assert!(res_select["error"].is_null(), "device_select failed: {:?}", res_select);

    // c) filesystem_read (with underscore)
    let operation = uuid::Uuid::new_v4();
    let (status, res_read, _) = send(
        &app,
        "POST",
        "/mcp",
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "filesystem_read",
                "arguments": {"path": "test.txt"},
                "_meta": {"runtimeOperationId": operation}
            }
        }),
        None,
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", res_read);
    assert!(res_read["error"].is_null(), "filesystem_read failed: {:?}", res_read);
    assert_eq!(res_read["result"]["structuredContent"]["status"], "pending");
}

