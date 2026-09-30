use async_trait::async_trait;
use moka::future::Cache;
use runtime_domain::*;
use runtime_ports::*;
use std::{
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
                .max_capacity(capacity)
                .time_to_live(ttl)
                .build(),
            metrics: CacheMetrics::default(),
        }
    }
}
/// Pure compiler. Raw logs stay in storage; budget is UTF-8 bytes, never guessed tokens.
pub fn compile_events(
    workspace: WorkspaceId,
    version: u64,
    events: &[Event],
    budget: usize,
) -> String {
    let mut out = format!("workspace={workspace}; graph_version={version}\n");
    let mut prioritized: Vec<_> = events.iter().collect();
    prioritized.sort_by_key(|e| {
        (
            match e.kind.as_str() {
                "Failed" | "TimedOut" => 0,
                "mutated" | "decision" => 1,
                _ => 2,
            },
            std::cmp::Reverse(e.created_at),
        )
    });
    for e in prioritized {
        let line = format!(
            "{} operation={} event={}: {}\n",
            e.kind, e.operation_id, e.id, e.summary
        );
        if out.len() + line.len() > budget {
            continue;
        }
        out.push_str(&line);
    }
    if out.len() > budget {
        let mut end = budget.min(out.len());
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
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
        let budget = budget.clamp(128, 128 * 1024);
        for _ in 0..5 {
            let ws = self.store.get_workspace(workspace).await?;
            let key = format!(
                "ctx:{workspace}:{}:{operation:?}:{budget}",
                ws.graph_version
            );
            let context = if let Some(c) = self.cache.get(&key).await {
                self.metrics.hits.fetch_add(1, Ordering::Relaxed);
                c
            } else {
                self.metrics.misses.fetch_add(1, Ordering::Relaxed);
                let events = match operation {
                    Some(op) => self.store.causal(workspace, op, 256).await?,
                    None => self.store.recent(workspace, 256).await?,
                };
                let c = compile_events(workspace, ws.graph_version, &events, budget);
                self.cache.insert(key, c.clone()).await;
                c
            };
            if self.store.get_workspace(workspace).await?.graph_version == ws.graph_version {
                return Ok(context);
            }
        }
        Err(PortError::Busy)
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
