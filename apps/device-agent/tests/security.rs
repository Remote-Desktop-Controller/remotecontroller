use device_agent::{validate_reply, validate_server};
use runtime_protocol::{Metadata, Request, RequestEnvelope, ResponseEnvelope};

#[test]
fn remote_server_requires_https_even_with_loopback_test_opt_in() {
    assert!(validate_server("http://remote.example", false).is_err());
    assert!(validate_server("http://remote.example", true).is_err());
    assert!(validate_server("https://remote.example", false).is_ok());
}

#[test]
fn loopback_http_requires_explicit_opt_in() {
    assert!(validate_server("http://127.0.0.1:8080", false).is_err());
    assert!(validate_server("http://127.0.0.1:8080", true).is_ok());
    assert!(validate_server("http://[::1]:8080", true).is_ok());
}

#[test]
fn service_url_never_contains_credentials_query_or_fragment() {
    for url in [
        "https://token@remote.example",
        "https://remote.example?token=x",
        "https://remote.example/#x",
    ] {
        assert!(validate_server(url, false).is_err(), "{url}");
    }
}

#[test]
fn replies_are_bound_to_operation_and_workspace() {
    let request = RequestEnvelope {
        meta: Metadata::new(Some(uuid::Uuid::new_v4()), 1000),
        payload: Request::Catalog,
    };
    let mut reply = ResponseEnvelope {
        meta: request.meta.clone(),
        payload: Ok(serde_json::json!({})),
    };
    assert!(validate_reply(&request, &reply).is_ok());
    reply.meta.operation_id = uuid::Uuid::new_v4();
    assert!(validate_reply(&request, &reply).is_err());
    reply.meta = request.meta.clone();
    reply.meta.workspace_id = Some(uuid::Uuid::new_v4());
    assert!(validate_reply(&request, &reply).is_err());
}

#[test]
fn profile_inside_workspace_is_rejected_before_creating_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let profile = workspace.join("private");
    assert!(device_agent::profile::protect_profile(&profile, &workspace).is_err());
    assert!(
        !profile.exists(),
        "reject before creating/modifying an in-workspace profile"
    );
}

#[test]
fn private_files_cannot_be_replaced_or_redirected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential.json");
    device_agent::profile::write_private(&path, b"first").unwrap();
    assert!(device_agent::profile::write_private(&path, b"second").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"first");
}

#[test]
fn remote_output_limit_preserves_identity_and_points_to_durable_local_operation() {
    let reply = ResponseEnvelope {
        meta: Metadata::new(Some(uuid::Uuid::new_v4()), 1000),
        payload: Ok(serde_json::json!({"content":"x".repeat(1024*1024)})),
    };
    let operation = reply.meta.operation_id;
    let bounded = device_agent::bounded_result(reply).unwrap();
    assert_eq!(bounded.meta.operation_id, operation);
    assert!(
        bounded.payload.is_err(),
        "oversized raw output must not block outbox delivery forever"
    );
}
