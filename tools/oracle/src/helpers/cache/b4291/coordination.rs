use std::collections::BTreeMap;
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::Ttl;
use betterlite_cache::coordinator::{
    AsyncCacheProtocol, CoordinatorError, EventDrivenCacheCoordinator,
};
use betterlite_context::local_ttl_cache::CacheValue;
use serde_json::json;
use thiserror::Error;
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};

use super::super::b4290::{failure, runtime};
use crate::helpers::HelperResult;

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

const SETTLE_ROUNDS: usize = 8;

const PROBE_KEY: &str = "key";

#[derive(Debug, Error)]
enum ProbeError {
    #[error("the probe load failed")]
    LoadFailed,
}

#[derive(Default)]
struct ProbeCache {
    values: Mutex<BTreeMap<String, CacheValue>>,
}

#[async_trait]
impl AsyncCacheProtocol for ProbeCache {
    type Error = ProbeError;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, ProbeError> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let stored = values.get(key).cloned();
        drop(values);
        Ok(stored)
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        _ttl: Ttl,
    ) -> Result<(), ProbeError> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.insert(key.to_owned(), value);
        drop(values);
        Ok(())
    }
}

struct Probe {
    cache: Arc<ProbeCache>,
    coordinator: Arc<EventDrivenCacheCoordinator>,
    loads: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
    value: CacheValue,
}

impl Probe {
    fn new(value: &CacheValue) -> Self {
        Self {
            cache: Arc::new(ProbeCache::default()),
            coordinator: Arc::new(EventDrivenCacheCoordinator::new(
                "[CACHE]".to_owned(),
                WAIT_TIMEOUT,
            )),
            loads: Arc::new(AtomicUsize::new(0)),
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            value: value.clone(),
        }
    }
}

type Outcome = Result<Option<CacheValue>, CoordinatorError<ProbeError, ProbeError>>;

async fn counted_load(probe: Arc<Probe>) -> Result<CacheValue, ProbeError> {
    probe.loads.fetch_add(1, Ordering::SeqCst);
    probe.started.notify_one();
    probe.release.notified().await;
    Ok(probe.value.clone())
}

fn unblocked_value(probe: &Probe) -> CacheValue {
    probe.loads.fetch_add(1, Ordering::SeqCst);
    probe.value.clone()
}

async fn failing_load(probe: Arc<Probe>) -> Result<CacheValue, ProbeError> {
    probe.started.notify_one();
    probe.release.notified().await;
    Err(ProbeError::LoadFailed)
}

async fn run_blocking_request(probe: Arc<Probe>) -> Outcome {
    let cache = Arc::clone(&probe.cache);
    let coordinator = Arc::clone(&probe.coordinator);
    let loader = Arc::clone(&probe);
    coordinator
        .get_or_load(PROBE_KEY, cache.as_ref(), move || counted_load(loader))
        .await
}

async fn run_unblocked_request(probe: Arc<Probe>) -> Outcome {
    let cache = Arc::clone(&probe.cache);
    let coordinator = Arc::clone(&probe.coordinator);
    let loader = Arc::clone(&probe);
    let load = move || ready(Ok::<_, ProbeError>(unblocked_value(&loader)));
    coordinator
        .get_or_load(PROBE_KEY, cache.as_ref(), load)
        .await
}

async fn run_failing_request(probe: Arc<Probe>) -> Outcome {
    let cache = Arc::clone(&probe.cache);
    let coordinator = Arc::clone(&probe.coordinator);
    let loader = Arc::clone(&probe);
    coordinator
        .get_or_load(PROBE_KEY, cache.as_ref(), move || failing_load(loader))
        .await
}

// a current-thread runtime runs each ready task on every yield, so eight yields settle every request
async fn settle() {
    for _ in 0..SETTLE_ROUNDS {
        yield_now().await;
    }
}

async fn collect(
    tasks: &mut JoinSet<Outcome>,
) -> Result<Vec<Option<CacheValue>>, Box<crate::helpers::HelperError>> {
    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        results.push(joined.map_err(failure)?.map_err(failure)?);
    }
    Ok(results)
}

pub(crate) fn single_loader(value: &CacheValue, requests: usize) -> HelperResult {
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async move {
        let probe = Arc::new(Probe::new(value));
        let mut tasks = JoinSet::new();
        tasks.spawn(run_blocking_request(Arc::clone(&probe)));
        probe.started.notified().await;
        for _ in 0..requests.saturating_sub(1) {
            tasks.spawn(run_unblocked_request(Arc::clone(&probe)));
        }
        settle().await;
        probe.release.notify_one();
        let results = collect(&mut tasks).await?;
        Ok(json!({"loads": probe.loads.load(Ordering::SeqCst), "results": results}))
    });
    outcome
}

pub(crate) fn waiter_gets_cached(value: &CacheValue) -> HelperResult {
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async move {
        let probe = Arc::new(Probe::new(value));
        let mut tasks = JoinSet::new();
        tasks.spawn(run_blocking_request(Arc::clone(&probe)));
        probe.started.notified().await;
        tasks.spawn(run_unblocked_request(Arc::clone(&probe)));
        settle().await;
        probe.release.notify_one();
        let results = collect(&mut tasks).await?;
        let cached = probe
            .cache
            .async_get_cache(PROBE_KEY)
            .await
            .map_err(failure)?;
        Ok(json!({
            "loads": probe.loads.load(Ordering::SeqCst),
            "results": results,
            "cached": cached,
        }))
    });
    outcome
}

pub(crate) fn loader_failure(value: &CacheValue) -> HelperResult {
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async move {
        let probe = Arc::new(Probe::new(value));
        let mut tasks = JoinSet::new();
        tasks.spawn(run_failing_request(Arc::clone(&probe)));
        probe.started.notified().await;
        tasks.spawn(run_unblocked_request(Arc::clone(&probe)));
        settle().await;
        probe.release.notify_one();
        let mut failed = false;
        let mut waited = None;
        while let Some(joined) = tasks.join_next().await {
            match joined.map_err(failure)? {
                Err(CoordinatorError::Load { .. }) => failed = true,
                Ok(stored) => waited = stored,
                Err(other) => return Err(failure(other)),
            }
        }
        let retried = run_unblocked_request(Arc::clone(&probe))
            .await
            .map_err(failure)?;
        Ok(json!({
            "failed": failed,
            "waiter": waited,
            "retry": retried,
            "loads": probe.loads.load(Ordering::SeqCst),
        }))
    });
    outcome
}
