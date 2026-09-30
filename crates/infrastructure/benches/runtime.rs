use criterion::{Criterion, criterion_group, criterion_main};
use futures_util::StreamExt;
use runtime_domain::*;
use runtime_infrastructure::{
    config::RuntimeConfig,
    context::{ContextEngine, compile_events},
    files::{FileLimits, WorkspaceFiles},
    kernel::Kernel,
    server::serve,
    storage::LocalStore,
};
use runtime_ports::*;
use runtime_protocol::*;
use runtime_transport::{Client, Listener};
use std::hint::black_box;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

fn benchmarks(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    for i in 0..1000 {
        std::fs::write(root.path().join(format!("f{i}.txt")), vec![b'x'; 64]).unwrap();
    }
    let files = WorkspaceFiles::open(root.path(), FileLimits::default()).unwrap();
    let store = Arc::new(
        rt.block_on(LocalStore::open(&state.path().join("bench.db")))
            .unwrap(),
    );
    let workspace = rt.block_on(store.open_workspace("benchmark")).unwrap();
    let op = Operation {
        id: OperationId::new(),
        workspace_id: workspace.id,
        task_id: None,
        trace_id: TraceId::new(),
        tool: "benchmark".into(),
        input: b"{}".to_vec(),
        status: OperationStatus::Queued,
        result: None,
        created_at: now_ms(),
    };
    rt.block_on(store.claim(&op)).unwrap();
    rt.block_on(store.set_running(op.id)).unwrap();
    rt.block_on(store.finish(
        op.id,
        OperationStatus::Succeeded,
        &ToolResult {
            output: b"null".to_vec(),
            error: None,
        },
    ))
    .unwrap();
    let cache = ContextEngine::new(store.clone(), 256, Duration::from_secs(3600));
    rt.block_on(cache.put("hot".into(), "cached value".into()));
    c.bench_function("cache_hit", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(cache.get("hot").await) })
    });
    c.bench_function("cache_miss", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(cache.get("absent").await) })
    });
    c.bench_function("event_insert", |b| {
        b.to_async(&rt).iter(|| async {
            let event = Event {
                id: EventId::new(),
                workspace_id: workspace.id,
                operation_id: op.id,
                kind: "benchmark".into(),
                summary: "raw reference preserved".into(),
                raw: vec![7; 32],
                created_at: now_ms(),
            };
            store
                .append(&event, None, EdgeKind::Produced)
                .await
                .unwrap();
        })
    });
    c.bench_function("causal_query", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(store.causal(workspace.id, op.id, 128).await.unwrap()) })
    });
    c.bench_function("file_scan_1000", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                files
                    .scan(".", 2000, CancellationToken::new())
                    .await
                    .unwrap(),
            )
        })
    });
    c.bench_function("batch_read_100", |b| {
        b.to_async(&rt).iter(|| async {
            let results: Vec<_> = futures_util::stream::iter(0..100)
                .map(|i| {
                    let files = files.clone();
                    async move { files.read(&format!("f{i}.txt")).await.unwrap() }
                })
                .buffer_unordered(16)
                .collect()
                .await;
            black_box(results)
        })
    });
    let events = rt.block_on(store.recent(workspace.id, 128)).unwrap();
    c.bench_function("context_compilation_128", |b| {
        b.iter(|| black_box(compile_events(workspace.id, 1, &events, 8192)))
    });
    let ipc_state = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\rdc-bench-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = ipc_state
        .path()
        .join("bench.sock")
        .to_string_lossy()
        .to_string();
    let kernel = rt
        .block_on(Kernel::open(
            root.path(),
            ipc_state.path(),
            RuntimeConfig::default(),
            ExecutionPolicy::read_only(),
        ))
        .unwrap();
    let stop = CancellationToken::new();
    let listener = rt.block_on(Listener::bind(&endpoint)).unwrap();
    let server = rt.spawn(serve(listener, kernel, vec![33; 32], stop.clone()));
    let client = Client::new(endpoint, vec![33; 32]);
    let heartbeat = RequestEnvelope {
        meta: Metadata::new(None, 3_600_000),
        payload: Request::Heartbeat,
    };
    c.bench_function("ipc_round_trip_with_handshake", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(client.request(&heartbeat, |_| {}).await.unwrap()) })
    });
    stop.cancel();
    rt.block_on(server).unwrap().unwrap();
}
criterion_group! {name=benches;config=Criterion::default().sample_size(10).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(2));targets=benchmarks}
criterion_main!(benches);
