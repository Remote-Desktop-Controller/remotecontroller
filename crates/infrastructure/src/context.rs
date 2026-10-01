use async_trait::async_trait;
use moka::future::Cache;
use runtime_domain::*;
use runtime_ports::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
pub struct CacheMetrics {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
}
#[derive(Clone, Debug)]
pub struct ContextRequest {
    pub workspace: WorkspaceId,
    pub operation: Option<OperationId>,
    pub task_id: Option<TaskId>,
    pub query: String,
    pub budget: usize,
}
pub struct ContextEngine<S: Storage> {
    store: Arc<S>,
    cache: Cache<String, String>,
    pub metrics: CacheMetrics,
}
impl<S: Storage> ContextEngine<S> {
    pub fn new(store: Arc<S>, capacity: u64, ttl: Duration) -> Self {
        Self {
            store,
            cache: Cache::builder()
                .max_capacity(capacity.clamp(1, 512) * (132 * 1024))
                .time_to_live(ttl)
                .weigher(|key: &String, value: &String| {
                    // Charge at least one full context slot, bounding entry overhead as well as bytes.
                    (key.len().saturating_add(value.len())).clamp(132 * 1024, u32::MAX as usize)
                        as u32
                })
                .build(),
            metrics: CacheMetrics::default(),
        }
    }
    pub async fn compile_request(&self, mut request: ContextRequest) -> Result<String> {
        request.budget = request.budget.min(128 * 1024);
        // Reject oversized options instead of aliasing different queries in the cache.
        if request.query.len() > 4096 {
            return Err(PortError::Policy(
                "context query exceeds 4096 UTF-8 bytes".into(),
            ));
        }
        let tokens: Vec<_> = request
            .query
            .split_whitespace()
            .take(32)
            .map(str::to_lowercase)
            .collect();
        for _ in 0..5 {
            let ws = self.store.get_workspace(request.workspace).await?;
            let key = format!(
                "ctx:{}:{}:{:?}:{:?}:{}:{:?}",
                request.workspace,
                ws.graph_version,
                request.operation,
                request.task_id,
                request.budget,
                request.query
            );
            let context = if let Some(c) = self.cache.get(&key).await {
                self.metrics.hits.fetch_add(1, Ordering::Relaxed);
                c
            } else {
                self.metrics.misses.fetch_add(1, Ordering::Relaxed);
                let mut events = if let Some(task) = request.task_id {
                    self.store.task_events(request.workspace, task, 256).await?
                } else {
                    self.store.recent(request.workspace, 256).await?
                };
                let mut causal_ids = HashSet::new();
                if let Some(op) = request.operation {
                    let causal = self.store.causal(request.workspace, op, 256).await?;
                    causal_ids.extend(causal.iter().map(|e| e.id));
                    let mut seen: HashSet<_> = events.iter().map(|e| e.id).collect();
                    for e in causal {
                        if seen.insert(e.id) {
                            events.push(e);
                        }
                    }
                }
                let mut operations = HashMap::new();
                for event in &events {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        operations.entry(event.operation_id)
                    {
                        let op = self.store.operation(event.operation_id).await?;
                        if op.workspace_id != request.workspace {
                            return Err(PortError::Policy(
                                "cross-workspace context reference".into(),
                            ));
                        }
                        entry.insert(op);
                    }
                }
                events.sort_by_key(|e| {
                    let op = &operations[&e.operation_id];
                    let haystack = format!(
                        "{} {} {}",
                        e.summary,
                        op.tool,
                        String::from_utf8_lossy(&op.input)
                    )
                    .to_lowercase();
                    let matches = tokens
                        .iter()
                        .filter(|t| haystack.contains(t.as_str()))
                        .count();
                    (
                        std::cmp::Reverse(
                            request.task_id.is_some() && op.task_id == request.task_id,
                        ),
                        std::cmp::Reverse(matches),
                        priority(e),
                        std::cmp::Reverse(causal_ids.contains(&e.id)),
                        std::cmp::Reverse(e.created_at),
                    )
                });
                let mut out = format!(
                    "workspace={}; graph_version={}\n",
                    request.workspace, ws.graph_version
                );
                let mut rolled_up = HashSet::new();
                for event in &events {
                    let line = event_line(event);
                    if out.len() + line.len() > request.budget {
                        continue;
                    }
                    out.push_str(&line);
                    let op = &operations[&event.operation_id];
                    if rolled_up.contains(&op.id) {
                        continue;
                    }
                    let raw = self.store.raw_event(request.workspace, event.id).await?;
                    let raw_data: Option<serde_json::Value> = serde_json::from_slice(&raw.raw).ok();
                    let is_raw = raw_data
                        .as_ref()
                        .is_some_and(|v| v.get("process_id").is_some());
                    let data = if is_raw {
                        raw_data
                    } else {
                        op.result
                            .as_ref()
                            .and_then(|r| serde_json::from_slice(&r.output).ok())
                    };
                    if let Some(data) = data {
                        let rollup = process_rollup(event, &data, is_raw);
                        if out.len() + rollup.len() <= request.budget {
                            out.push_str(&rollup);
                            if !rollup.is_empty() {
                                rolled_up.insert(op.id);
                            }
                        }
                    }
                }
                truncate_utf8(&mut out, request.budget);
                self.cache.insert(key, out.clone()).await;
                out
            };
            if self
                .store
                .get_workspace(request.workspace)
                .await?
                .graph_version
                == ws.graph_version
            {
                return Ok(context);
            }
        }
        Err(PortError::Busy)
    }
}
fn priority(event: &Event) -> u8 {
    match event.kind.as_str() {
        "Failed" | "TimedOut" => 0,
        "mutated" | "decision" => 1,
        _ => 2,
    }
}
fn event_line(e: &Event) -> String {
    format!(
        "{} operation={} event={}: {}\n",
        e.kind, e.operation_id, e.id, e.summary
    )
}
fn truncate_utf8(out: &mut String, budget: usize) {
    let mut end = budget.min(out.len());
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
}
fn process_rollup(event: &Event, data: &serde_json::Value, is_raw: bool) -> String {
    if data.get("process_id").is_none() {
        return String::new();
    }
    let fields = [
        "process_id",
        "status",
        "duration_ms",
        "stdout_bytes",
        "stderr_bytes",
        "exit_code",
        "truncated",
    ];
    let values = fields
        .into_iter()
        .filter_map(|key| data.get(key).map(|v| format!("{key}={v}")))
        .collect::<Vec<_>>()
        .join(" ");
    let reference = if is_raw {
        format!("raw_event={}", event.id)
    } else {
        format!("raw_operation={}", event.operation_id)
    };
    format!(
        "process_rollup {values} {reference} operation={}\n",
        event.operation_id
    )
}
/// Pure compiler; the budget counts UTF-8 bytes and raw references remain available.
pub fn compile_events(
    workspace: WorkspaceId,
    version: u64,
    events: &[Event],
    budget: usize,
) -> String {
    let mut out = format!("workspace={workspace}; graph_version={version}\n");
    let mut ordered: Vec<_> = events.iter().collect();
    ordered.sort_by_key(|e| (priority(e), std::cmp::Reverse(e.created_at)));
    for e in ordered {
        let line = event_line(e);
        if out.len() + line.len() <= budget {
            out.push_str(&line);
        }
    }
    truncate_utf8(&mut out, budget);
    out
}
#[async_trait]
impl<S: Storage + 'static> ContextStoragePort for ContextEngine<S> {
    async fn compile(
        &self,
        workspace: WorkspaceId,
        operation: Option<OperationId>,
        budget: usize,
    ) -> Result<String> {
        self.compile_request(ContextRequest {
            workspace,
            operation,
            task_id: None,
            query: String::new(),
            budget,
        })
        .await
    }
}
#[async_trait]
impl<S: Storage + 'static> CachePort for ContextEngine<S> {
    async fn get(&self, key: &str) -> Option<String> {
        self.cache.get(key).await
    }
    async fn put(&self, key: String, value: String) {
        self.cache.insert(key, value).await;
    }
    async fn invalidate(&self, key: &str) {
        self.cache.invalidate(key).await;
    }
}
