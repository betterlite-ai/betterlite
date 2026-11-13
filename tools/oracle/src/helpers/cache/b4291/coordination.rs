use std::collections::BTreeMap;
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
use crate::helpers::{HelperError, HelperResult};

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

const SETTLE_ROUNDS: usize = 8;

const PROBE_KEY: &str = "key";

type Failure = Box<HelperError>;

type Outcome = Result<Option<CacheValue>, CoordinatorError<ProbeError, ProbeError>>;

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

#[derive(Clone, Copy)]
enum LoadKind {
    Blocked,
    Unblocked,
    Failing,
}

struct Request {
    probe: Arc<Probe>,
    kind: LoadKind,
}

async fn probe_load(request: Arc<Request>) -> Result<CacheValue, ProbeError> {
    request.probe.loads.fetch_add(1, Ordering::SeqCst);
    match request.kind {
        LoadKind::Blocked => {
            request.probe.started.notify_one();
            request.probe.release.notified().await;
            Ok(request.probe.value.clone())
        }
        LoadKind::Unblocked => Ok(request.probe.value.clone()),
        LoadKind::Failing => {
            request.probe.started.notify_one();
            request.probe.release.notified().await;
            Err(ProbeError::LoadFailed)
        }
    }
}

async fn run_request(request: Arc<Request>) -> Outcome {
    let cache = Arc::clone(&request.probe.cache);
    let coordinator = Arc::clone(&request.probe.coordinator);
    let load = move || probe_load(request);
    coordinator
        .get_or_load(PROBE_KEY, cache.as_ref(), load)
        .await
}

// a current-thread runtime runs each ready task on every yield, so eight yields settle every request
async fn settle() {
    for _ in 0..SETTLE_ROUNDS {
        yield_now().await;
    }
}

fn request(probe: &Arc<Probe>, kind: LoadKind) -> Arc<Request> {
    Arc::new(Request {
        probe: Arc::clone(probe),
        kind,
    })
}

async fn start_requests(
    probe: &Arc<Probe>,
    tasks: &mut JoinSet<Outcome>,
    first: LoadKind,
    waiters: usize,
) {
    tasks.spawn(run_request(request(probe, first)));
    probe.started.notified().await;
    for _ in 0..waiters {
        tasks.spawn(run_request(request(probe, LoadKind::Unblocked)));
    }
    settle().await;
    probe.release.notify_one();
}

async fn collect(tasks: &mut JoinSet<Outcome>) -> Result<Vec<Option<CacheValue>>, Failure> {
    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        results.push(joined.map_err(failure)?.map_err(failure)?);
    }
    Ok(results)
}

async fn single_loader(probe: &Arc<Probe>, requests: usize) -> Result<serde_json::Value, Failure> {
    let mut tasks = JoinSet::new();
    let waiters = requests.saturating_sub(1);
    start_requests(probe, &mut tasks, LoadKind::Blocked, waiters).await;
    let results = collect(&mut tasks).await?;
    Ok(json!({"loads": probe.loads.load(Ordering::SeqCst), "results": results}))
}

async fn waiter_gets_cached(probe: &Arc<Probe>) -> Result<serde_json::Value, Failure> {
    let mut tasks = JoinSet::new();
    start_requests(probe, &mut tasks, LoadKind::Blocked, 1).await;
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
}

async fn loader_failure(probe: &Arc<Probe>) -> Result<serde_json::Value, Failure> {
    let mut tasks = JoinSet::new();
    start_requests(probe, &mut tasks, LoadKind::Failing, 1).await;
    let mut failed = false;
    let mut waited = None;
    while let Some(joined) = tasks.join_next().await {
        match joined.map_err(failure)? {
            Err(CoordinatorError::Load { .. }) => failed = true,
            Ok(stored) => waited = stored,
            Err(other) => return Err(failure(other)),
        }
    }
    let retried = run_request(request(probe, LoadKind::Unblocked))
        .await
        .map_err(failure)?;
    Ok(json!({
        "failed": failed,
        "waiter": waited,
        "retry": retried,
        "loads": probe.loads.load(Ordering::SeqCst),
    }))
}

pub(crate) enum Scenario {
    SingleLoader { requests: usize },
    WaiterGetsCached,
    LoaderFailure,
}

pub(crate) fn run_scenario(value: &CacheValue, scenario: Scenario) -> HelperResult {
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async move {
        let probe = Arc::new(Probe::new(value));
        match scenario {
            Scenario::SingleLoader { requests } => single_loader(&probe, requests).await,
            Scenario::WaiterGetsCached => waiter_gets_cached(&probe).await,
            Scenario::LoaderFailure => loader_failure(&probe).await,
        }
    });
    outcome
}
