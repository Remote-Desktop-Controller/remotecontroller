use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::{Value, json};
use std::sync::Arc;
async fn call(k: &Arc<Kernel>, tool: &str, input: Value, meta: Metadata) -> Value {
    k.handle(
        RequestEnvelope {
            meta,
            payload: Request::Execute {
                tool: tool.into(),
                input,
            },
        },
        Arc::new(NoProgress),
    )
    .await
    .payload
    .unwrap()
}
#[tokio::test]
async fn directory_move_remove_and_retry_preserve_user_data() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]),
    )
    .await
    .unwrap();
    let meta = Metadata::new(Some(k.workspace.id.as_uuid()), 10000);
    let made = call(
        &k,
        "filesystem.mkdir",
        json!({"path":"folder"}),
        meta.clone(),
    )
    .await;
    assert_eq!(made["status"], "Succeeded", "{made}");
    assert!(root.path().join("folder").is_dir());
    std::fs::write(root.path().join("source"), "precious").unwrap();
    let meta_move = Metadata::new(Some(k.workspace.id.as_uuid()), 10000);
    let moved = call(
        &k,
        "filesystem.move",
        json!({"source":"source","destination":"folder/moved.txt"}),
        meta_move.clone(),
    )
    .await;
    assert_eq!(moved["status"], "Succeeded", "{moved}");
    assert!(!root.path().join("source").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("folder/moved.txt")).unwrap(),
        "precious"
    );
    std::fs::write(root.path().join("source"), "new-external").unwrap();
    assert_eq!(
        call(
            &k,
            "filesystem.move",
            json!({"source":"source","destination":"folder/moved.txt"}),
            meta_move
        )
        .await,
        moved
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("source")).unwrap(),
        "new-external"
    );
    assert_eq!(
        call(&k, "filesystem.mkdir", json!({"path":"folder"}), meta).await,
        made
    );
    let denied = call(
        &k,
        "filesystem.move",
        json!({"source":"source","destination":"folder/moved.txt"}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await;
    assert_eq!(denied["status"], "Failed");
    let removed = call(
        &k,
        "filesystem.remove",
        json!({"path":"source"}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await;
    assert_eq!(removed["status"], "Succeeded");
    assert!(!root.path().join("source").exists());
    let root_remove = call(
        &k,
        "filesystem.remove",
        json!({"path":"."}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await;
    assert_eq!(root_remove["status"], "Failed");
    assert!(root.path().exists());
}
