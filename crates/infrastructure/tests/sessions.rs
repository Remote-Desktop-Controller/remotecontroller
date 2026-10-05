use runtime_domain::*;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel};
use runtime_ports::*;
use runtime_protocol::*;
use serde_json::{Value, json};
use std::sync::Arc;

#[tokio::test]
async fn decisions_and_pending_work_survive_restart_without_duplicate_notes() {
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
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = ExecutionPolicy::read_only();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        policy.clone(),
    )
    .await
    .unwrap();
    let metadata = || Metadata::new(Some(k.workspace.id.as_uuid()), 30000);
    let created = call(
        &k,
        "session.create",
        json!({"label":"Delivery plan"}),
        metadata(),
    )
    .await;
    let session = created["result"]["session_id"].as_str().unwrap().to_owned();
    let text = "Decision: preserve user files. Pending: verify public HTTPS.";
    let note_meta = metadata();
    let input = json!({"session_id":session,"text":text});
    let written = call(&k, "session.note", input.clone(), note_meta.clone()).await;
    assert_eq!(written["status"], "Succeeded", "{written}");
    let indexed = call(
        &k,
        "context.search",
        json!({"session_id":session,"query":"Pending"}),
        metadata(),
    )
    .await;
    assert!(
        indexed["result"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["summary"] == text)
    );
    call(
        &k,
        "session.close",
        json!({"session_id":session}),
        metadata(),
    )
    .await;
    drop(k);
    let k = Kernel::open(root.path(), state.path(), RuntimeConfig::default(), policy)
        .await
        .unwrap();
    assert_eq!(call(&k, "session.note", input, note_meta).await, written);
    let notes = call(
        &k,
        "session.notes",
        json!({"session_id":session}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 30000),
    )
    .await;
    assert_eq!(notes["result"]["notes"].as_array().unwrap().len(), 1);
    assert_eq!(notes["result"]["notes"][0]["text"], text);
}

#[tokio::test]
async fn durable_memory_tools_are_in_the_catalogue() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let names: Vec<_> = kernel
        .executor
        .catalogue()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for tool in [
        "session.create",
        "session.list",
        "session.resume",
        "session.close",
        "session.note",
        "session.notes",
        "context.search",
        "process.output",
        "process.search",
    ] {
        assert!(
            names.iter().any(|name| name == tool),
            "missing durable memory tool: {tool}"
        );
    }
}

async fn call(kernel: &Arc<Kernel>, tool: &str, input: Value, meta: Metadata) -> ResponseEnvelope {
    kernel
        .handle(
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
}

fn bound(kernel: &Kernel, session: &str) -> Metadata {
    let mut value =
        serde_json::to_value(Metadata::new(Some(kernel.workspace.id.as_uuid()), 10000)).unwrap();
    value["session_id"] = json!(session);
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn sessions_restart_close_resume_and_retry_binding_are_durable() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "original").unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let created = call(
        &k,
        "session.create",
        json!({"label":"investigation"}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    let session = created["result"]["session_id"].as_str().unwrap().to_owned();
    let meta = bound(&k, &session);
    let first = call(&k, "filesystem.read", json!({"path":"file"}), meta.clone())
        .await
        .payload
        .unwrap();
    let other = call(
        &k,
        "session.create",
        json!({}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    let mut wrong = bound(&k, other["result"]["session_id"].as_str().unwrap());
    wrong.operation_id = meta.operation_id;
    let conflict = call(&k, "filesystem.read", json!({"path":"file"}), wrong)
        .await
        .payload
        .unwrap_err();
    assert_eq!(conflict.code, ErrorCode::Conflict);
    call(
        &k,
        "session.close",
        json!({"session_id":session}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    let retry = call(&k, "filesystem.read", json!({"path":"file"}), meta.clone())
        .await
        .payload
        .unwrap();
    assert_eq!(first, retry);
    let blocked = call(
        &k,
        "filesystem.read",
        json!({"path":"file"}),
        bound(&k, &session),
    )
    .await
    .payload
    .unwrap_err();
    assert_eq!(blocked.code, ErrorCode::PolicyDenied);
    let root2 = tempfile::tempdir().unwrap();
    let k2 = Kernel::open(
        root2.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let foreign = call(
        &k2,
        "filesystem.read",
        json!({"path":"file"}),
        bound(&k2, &session),
    )
    .await
    .payload
    .unwrap_err();
    assert_eq!(foreign.code, ErrorCode::NotFound);
    drop(k2);
    drop(k);
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let list = call(
        &k,
        "session.list",
        json!({}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    assert!(
        list["result"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["session_id"] == session && s["status"] == "closed")
    );
    call(
        &k,
        "session.resume",
        json!({"session_id":session}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    let resumed = call(
        &k,
        "filesystem.read",
        json!({"path":"file"}),
        bound(&k, &session),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(resumed["status"], "Succeeded");
    let history = call(
        &k,
        "context.search",
        json!({"session_id":session,"query":"filesystem.read"}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    assert!(!history["result"]["events"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn history_search_finds_old_events_pages_stably_and_enforces_byte_budget() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: k.workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "fixture".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    k.services.store.claim(&op).await.unwrap();
    let mut expected = Vec::new();
    for index in 0..320 {
        let event = Event {
            id: EventId::new(),
            workspace_id: k.workspace.id,
            operation_id: op.id,
            kind: "fixture".into(),
            summary: format!(
                "{} {index}",
                if index < 3 { "old-needle" } else { "recent" }
            ),
            raw: b"{\"raw\":true}".to_vec(),
            created_at: index + 2,
        };
        if index < 3 {
            expected.push(event.id.to_string());
        }
        k.services
            .store
            .append(&event, None, EdgeKind::Produced)
            .await
            .unwrap();
    }
    let first = call(
        &k,
        "context.search",
        json!({"query":"old-needle","limit":2}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(first["events"].as_array().unwrap().len(), 2);
    assert_eq!(first["events"][0]["event_id"], expected[2]);
    assert!(first["next_cursor"].is_string());
    let second = call(
        &k,
        "context.search",
        json!({"query":"old-needle","limit":2,"cursor":first["next_cursor"]}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(second["events"][0]["event_id"], expected[0]);
    assert!(second["next_cursor"].is_null());
    let tiny = call(
        &k,
        "context.search",
        json!({"query":"old-needle","budget_bytes":0}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert!(tiny["events"].as_array().unwrap().is_empty());
    assert_eq!(tiny["truncated"], true);
    assert_eq!(tiny["bytes_used"], 0);
    let raw = call(
        &k,
        "context.event",
        json!({"event_id":expected[0]}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(raw["result"]["payload"]["raw"], true);
}

#[tokio::test]
async fn migration_upgrades_v02_database_without_discarding_operations() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("runtime.db");
    let ws = WorkspaceId::new();
    let op = OperationId::new();
    {
        let db = libsql::Builder::new_local(&path).build().await.unwrap();
        let connection = db.connect().unwrap();
        connection
            .execute_batch(include_str!("../../../migrations/001_initial.sql"))
            .await
            .unwrap();
        connection
            .execute("INSERT INTO schema_migrations VALUES(1,1)", ())
            .await
            .unwrap();
        connection
            .execute(
                "INSERT INTO workspaces(id,root,created_at,updated_at) VALUES(?1,'old',1,1)",
                [ws.to_string()],
            )
            .await
            .unwrap();
        connection.execute("INSERT INTO operations(id,workspace_id,trace_id,tool,input,status,output,created_at,updated_at) VALUES(?1,?2,?3,'old.tool',X'7b7d','Succeeded',X'3432',1,1)",libsql::params![op.to_string(),ws.to_string(),TraceId::new().to_string()]).await.unwrap();
    }
    let store = runtime_infrastructure::storage::LocalStore::open(&path)
        .await
        .unwrap();
    assert_eq!(
        store.operation(op).await.unwrap().result.unwrap().output,
        b"42"
    );
    drop(store);
    let store = runtime_infrastructure::storage::LocalStore::open(&path)
        .await
        .unwrap();
    assert_eq!(
        store.operation(op).await.unwrap().status,
        OperationStatus::Succeeded
    );
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let c = db.connect().unwrap();
    let mut rows = c
        .query("SELECT COUNT(*) FROM schema_migrations WHERE version=2", ())
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1
    );
}

#[tokio::test]
async fn process_ranges_search_and_restart_preserve_offsets_quotas_and_workspace_scope() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: k.workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "fixture".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: 1,
    };
    k.services.store.claim(&op).await.unwrap();
    let process = ProcessId::new();
    let db = libsql::Builder::new_local(state.path().join("runtime.db"))
        .build()
        .await
        .unwrap();
    let c = db.connect().unwrap();
    c.execute("INSERT INTO processes(id,workspace_id,operation_id,cwd,started_at,status) VALUES(?1,?2,?3,'.',1,'Exited')",libsql::params![process.to_string(),k.workspace.id.to_string(),op.id.to_string()]).await.unwrap();
    let spool = state.path().join("process-spool").join(process.to_string());
    std::fs::create_dir_all(&spool).unwrap();
    let bytes = "áneedle--needle--tail".as_bytes();
    std::fs::write(spool.join("stdout"), bytes).unwrap();
    std::fs::write(spool.join("stderr"), b"error").unwrap();
    std::fs::write(spool.join("metadata.json"),serde_json::to_vec(&json!({"stdout_bytes":bytes.len()+100,"stderr_bytes":5,"stdout_stored":bytes.len(),"stderr_stored":5,"truncated":true,"capture_incomplete":false,"duration_ms":1})).unwrap()).unwrap();
    let range = call(
        &k,
        "process.output",
        json!({"process_id":process.to_string(),"offset":1,"max_bytes":3}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(range["encoding"], "hex");
    assert_eq!(range["data"], "a16e65");
    assert_eq!(range["next_offset"], 4);
    assert_eq!(range["truncated"], true);
    assert_eq!(range["generated_bytes"], bytes.len() + 100);
    let search = call(
        &k,
        "process.search",
        json!({"process_id":process.to_string(),"query":"needle","limit":1,"context_bytes":2}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(search["matches"].as_array().unwrap().len(), 1);
    assert_eq!(search["matches"][0]["offset"], 2);
    let offset = search["next_offset"].as_u64().unwrap();
    let second=call(&k,"process.search",json!({"process_id":process.to_string(),"query":"needle","offset":offset,"context_bytes":2}),Metadata::new(Some(k.workspace.id.as_uuid()),10000)).await.payload.unwrap()["result"].clone();
    assert_eq!(second["matches"][0]["offset"], 10);
    let missing = call(
        &k,
        "process.output",
        json!({"process_id":process.to_string(),"offset":9999}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(missing["data"], "");
    assert!(missing["next_offset"].is_null());
    drop(k);
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let replay = call(
        &k,
        "process.output",
        json!({"process_id":process.to_string(),"offset":1,"max_bytes":3}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]
        .clone();
    assert_eq!(replay["data"], range["data"]);
    let root2 = tempfile::tempdir().unwrap();
    let k2 = Kernel::open(
        root2.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let foreign = call(
        &k2,
        "process.output",
        json!({"process_id":process.to_string()}),
        Metadata::new(Some(k2.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(foreign["status"], "Failed");
    assert_eq!(foreign["error"], "not found");
}

#[tokio::test]
async fn bound_mutation_retry_after_restart_does_not_repeat_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = ExecutionPolicy::new([Capability::ReadWorkspace, Capability::WriteWorkspace]);
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        policy.clone(),
    )
    .await
    .unwrap();
    let session = call(
        &k,
        "session.create",
        json!({"label":"mutation"}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let metadata = bound(&k, &session);
    let first = call(
        &k,
        "filesystem.write",
        json!({"path":"once","content":"first"}),
        metadata.clone(),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(first["status"], "Succeeded");
    std::fs::write(root.path().join("once"), "external-change").unwrap();
    drop(k);
    let k = Kernel::open(root.path(), state.path(), RuntimeConfig::default(), policy)
        .await
        .unwrap();
    let retry = call(
        &k,
        "filesystem.write",
        json!({"path":"once","content":"first"}),
        metadata,
    )
    .await
    .payload
    .unwrap();
    assert_eq!(first, retry);
    assert_eq!(
        std::fs::read_to_string(root.path().join("once")).unwrap(),
        "external-change"
    );
}

#[tokio::test]
async fn session_context_defaults_filter_search_compile_cache_and_causal_sources() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "content").unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let mut sessions = Vec::new();
    let mut operations = Vec::new();
    for marker in ["PRIVATE_ALPHA", "PRIVATE_BETA"] {
        let session = call(
            &k,
            "session.create",
            json!({"label":marker}),
            Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
        )
        .await
        .payload
        .unwrap()["result"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let op = call(
            &k,
            "filesystem.read",
            json!({"path":"file"}),
            bound(&k, &session),
        )
        .await
        .payload
        .unwrap()["operation_id"]
            .as_str()
            .unwrap()
            .parse::<OperationId>()
            .unwrap();
        k.services
            .store
            .append(
                &Event {
                    id: EventId::new(),
                    workspace_id: k.workspace.id,
                    operation_id: op,
                    kind: "decision".into(),
                    summary: marker.into(),
                    raw: b"{}".to_vec(),
                    created_at: now_ms(),
                },
                None,
                EdgeKind::Produced,
            )
            .await
            .unwrap();
        sessions.push(session);
        operations.push(op);
    }
    let search = call(&k, "context.search", json!({}), bound(&k, &sessions[0]))
        .await
        .payload
        .unwrap()["result"]
        .clone();
    assert!(
        search["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["session_id"] == sessions[0])
    );
    let alpha = call(
        &k,
        "context.compile",
        json!({"query":"PRIVATE_BETA"}),
        bound(&k, &sessions[0]),
    )
    .await
    .payload
    .unwrap()["result"]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(alpha.contains("PRIVATE_ALPHA"));
    assert!(!alpha.contains("PRIVATE_BETA"));
    let beta = call(&k, "context.compile", json!({}), bound(&k, &sessions[1]))
        .await
        .payload
        .unwrap()["result"]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(beta.contains("PRIVATE_BETA"));
    assert!(!beta.contains("PRIVATE_ALPHA"));
    let legacy = call(
        &k,
        "context.compile",
        json!({}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap()["result"]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(legacy.contains("PRIVATE_ALPHA") && legacy.contains("PRIVATE_BETA"));
    let causal = call(
        &k,
        "context.causal",
        json!({"operation_id":operations[1].to_string()}),
        bound(&k, &sessions[0]),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(causal["status"], "Failed");
    assert_eq!(causal["error"], "not found");
    let override_session = call(
        &k,
        "context.search",
        json!({"session_id":sessions[1]}),
        bound(&k, &sessions[0]),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(override_session["status"], "Failed");
}

#[tokio::test]
async fn bound_recent_and_raw_events_isolate_sessions_and_preserve_legacy_access() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let k = Kernel::open(
        root.path(),
        state.path(),
        RuntimeConfig::default(),
        ExecutionPolicy::read_only(),
    )
    .await
    .unwrap();
    let mut sessions = Vec::new();
    let mut event_ids = Vec::new();
    for marker in ["RECENT_ALPHA_NOTE", "RECENT_BETA_NOTE"] {
        let created = call(
            &k,
            "session.create",
            json!({"label": marker}),
            Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
        )
        .await
        .payload
        .unwrap();
        let session = created["result"]["session_id"].as_str().unwrap().to_owned();
        let written = call(
            &k,
            "session.note",
            json!({"session_id": session, "text": marker}),
            bound(&k, &session),
        )
        .await
        .payload
        .unwrap();
        assert_eq!(written["status"], "Succeeded", "{written}");
        event_ids.push(written["result"]["event_id"].as_str().unwrap().to_owned());
        sessions.push(session);
    }

    for (index, marker) in ["RECENT_ALPHA_NOTE", "RECENT_BETA_NOTE"].iter().enumerate() {
        let recent = call(
            &k,
            "context.recent",
            json!({"limit": 100}),
            bound(&k, &sessions[index]),
        )
        .await
        .payload
        .unwrap();
        assert_eq!(recent["status"], "Succeeded", "{recent}");
        let events = recent["result"]["events"].as_array().unwrap();
        assert!(events.iter().any(|event| event["summary"] == *marker));
        for event in events {
            let operation = event["operation_id"].as_str().unwrap().parse().unwrap();
            assert_eq!(
                k.services
                    .store
                    .operation_session(k.workspace.id, operation)
                    .await
                    .unwrap(),
                Some(sessions[index].parse().unwrap()),
                "event from another session returned: {event}"
            );
        }
        let own = call(
            &k,
            "context.event",
            json!({"event_id": event_ids[index]}),
            bound(&k, &sessions[index]),
        )
        .await
        .payload
        .unwrap();
        assert_eq!(own["status"], "Succeeded", "{own}");
        assert_eq!(own["result"]["summary"], *marker);
        let foreign = call(
            &k,
            "context.event",
            json!({"event_id": event_ids[1 - index]}),
            bound(&k, &sessions[index]),
        )
        .await
        .payload
        .unwrap();
        assert_eq!(foreign["status"], "Failed", "{foreign}");
        assert_eq!(foreign["error"], "not found");
        assert!(
            foreign["result"].is_null(),
            "foreign raw payload returned: {foreign}"
        );
    }

    let legacy = call(
        &k,
        "context.recent",
        json!({"limit": 100}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    for marker in ["RECENT_ALPHA_NOTE", "RECENT_BETA_NOTE"] {
        assert!(
            legacy["result"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["summary"] == marker)
        );
    }
    let legacy_raw = call(
        &k,
        "context.event",
        json!({"event_id": event_ids[1]}),
        Metadata::new(Some(k.workspace.id.as_uuid()), 10000),
    )
    .await
    .payload
    .unwrap();
    assert_eq!(legacy_raw["status"], "Succeeded", "{legacy_raw}");
    assert_eq!(legacy_raw["result"]["summary"], "RECENT_BETA_NOTE");
}
