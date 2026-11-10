#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinSet, yield_now};
use tracing::instrument::WithSubscriber as _;

use super::{AsyncCacheProtocol, CoordinatorError, EventDrivenCacheCoordinator};
use crate::backend::Ttl;

#[derive(Debug, Error)]
enum FakeError {
    #[error("the fake cache read failed")]
    ReadFailed,
    #[error("the fake load failed")]
    LoadFailed,
}

struct FakeCache {
    values: Mutex<BTreeMap<String, CacheValue>>,
    read_count: AtomicUsize,
    fail_reads: bool,
    fill_after_first_read: bool,
}

impl FakeCache {
    fn new(fail_reads: bool) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            read_count: AtomicUsize::new(0),
            fail_reads,
            fill_after_first_read: false,
        }
    }

    fn filling_after_first_read() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            read_count: AtomicUsize::new(0),
            fail_reads: false,
            fill_after_first_read: true,
        }
    }

    fn clear(&self) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn stored(&self, key: &str) -> Option<CacheValue> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl AsyncCacheProtocol for FakeCache {
    type Error = FakeError;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, FakeError> {
        if self.fail_reads {
            return Err(FakeError::ReadFailed);
        }
        let read = self.read_count.fetch_add(1, Ordering::SeqCst);
        if self.fill_after_first_read && read == 0 {
            self.async_set_cache(key, CacheValue::Integer(5), None)
                .await?;
            return Ok(None);
        }
        Ok(self.stored(key))
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        _ttl: Ttl,
    ) -> Result<(), FakeError> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        Ok(())
    }
}

type Outcome = Result<Option<CacheValue>, CoordinatorError<FakeError, FakeError>>;

#[derive(Clone, Copy)]
enum LoadKind {
    Blocked,
    Unblocked,
    Failing,
    Pending,
}

struct TestLoader {
    kind: LoadKind,
    loads: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

async fn run_load(loader: Arc<TestLoader>) -> Result<CacheValue, FakeError> {
    loader.loads.fetch_add(1, Ordering::SeqCst);
    match loader.kind {
        LoadKind::Blocked => {
            loader.started.notify_one();
            loader.release.notified().await;
            Ok(CacheValue::Integer(42))
        }
        LoadKind::Unblocked => Ok(CacheValue::Integer(42)),
        LoadKind::Failing => {
            loader.started.notify_one();
            loader.release.notified().await;
            Err(FakeError::LoadFailed)
        }
        LoadKind::Pending => {
            loader.started.notify_one();
            pending().await
        }
    }
}

struct Scenario {
    cache: Arc<FakeCache>,
    coordinator: Arc<EventDrivenCacheCoordinator>,
    loads: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl Scenario {
    fn new(wait_timeout: Duration) -> Self {
        Self {
            cache: Arc::new(FakeCache::new(false)),
            coordinator: Arc::new(EventDrivenCacheCoordinator::new(
                "[CACHE]".to_owned(),
                wait_timeout,
            )),
            loads: Arc::new(AtomicUsize::new(0)),
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    fn spawn(&self, tasks: &mut JoinSet<Outcome>, kind: LoadKind) -> AbortHandle {
        let loader = Arc::new(TestLoader {
            kind,
            loads: Arc::clone(&self.loads),
            started: Arc::clone(&self.started),
            release: Arc::clone(&self.release),
        });
        let cache = Arc::clone(&self.cache);
        let coordinator = Arc::clone(&self.coordinator);
        let load = move || run_load(loader);
        tasks.spawn(async move { coordinator.get_or_load("key", cache.as_ref(), load).await })
    }
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .flush()
    }
}

async fn captured_log(prefix: &str) -> String {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let cache = FakeCache::new(false);
    let coordinator = EventDrivenCacheCoordinator::new(prefix.to_owned(), Duration::from_secs(5));
    let loaded = coordinator
        .get_or_load("key", &cache, || async {
            Ok::<_, FakeError>(CacheValue::Integer(1))
        })
        .with_subscriber(subscriber)
        .await;
    loaded.unwrap();
    let log = bytes.lock().unwrap_or_else(PoisonError::into_inner).clone();
    String::from_utf8(log).unwrap()
}

fn coordinator(wait_timeout: Duration) -> Arc<EventDrivenCacheCoordinator> {
    Arc::new(EventDrivenCacheCoordinator::new(
        "[CACHE]".to_owned(),
        wait_timeout,
    ))
}

// a current-thread runtime runs each ready task on every yield, so eight yields settle both requests
async fn settle() {
    for _ in 0..8_usize {
        yield_now().await;
    }
}

#[tokio::test]
async fn cache_coordination_hit_skips_the_load() {
    let cache = FakeCache::new(false);
    cache
        .async_set_cache("key", CacheValue::Integer(7), None)
        .await
        .unwrap();
    let coordinator = coordinator(Duration::from_secs(5));
    let loads = AtomicUsize::new(0);
    let value = coordinator
        .get_or_load("key", &cache, || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, FakeError>(CacheValue::Integer(9))
        })
        .await
        .unwrap();
    assert_eq!(value, Some(CacheValue::Integer(7)));
    assert_eq!(loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cache_coordination_miss_loads_and_caches_the_value() {
    let cache = FakeCache::new(false);
    let coordinator = coordinator(Duration::from_secs(5));
    let value = coordinator
        .get_or_load("key", &cache, || async {
            Ok::<_, FakeError>(CacheValue::Integer(9))
        })
        .await
        .unwrap();
    assert_eq!(value, Some(CacheValue::Integer(9)));
    assert_eq!(cache.stored("key"), Some(CacheValue::Integer(9)));
}

#[tokio::test]
async fn cache_coordination_reports_a_cache_read_failure() {
    let cache = FakeCache::new(true);
    let coordinator = coordinator(Duration::from_secs(5));
    let value = coordinator
        .get_or_load("key", &cache, || async {
            Ok::<_, FakeError>(CacheValue::Integer(9))
        })
        .await;
    assert!(matches!(value, Err(CoordinatorError::Cache { .. })));
}

#[tokio::test]
async fn cache_coordination_double_check_returns_a_value_filled_while_claiming() {
    let cache = FakeCache::filling_after_first_read();
    let coordinator = coordinator(Duration::from_secs(5));
    let loads = AtomicUsize::new(0);
    let value = coordinator
        .get_or_load("key", &cache, || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, FakeError>(CacheValue::Integer(9))
        })
        .await
        .unwrap();
    assert_eq!(value, Some(CacheValue::Integer(5)));
    assert_eq!(loads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cache_coordination_resets_the_loader_state_between_loads() {
    let cache = FakeCache::new(false);
    let coordinator = coordinator(Duration::from_secs(5));
    let loads = AtomicUsize::new(0);
    let first = coordinator
        .get_or_load("key", &cache, || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, FakeError>(CacheValue::Integer(1))
        })
        .await
        .unwrap();
    cache.clear();
    let second = coordinator
        .get_or_load("key", &cache, || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, FakeError>(CacheValue::Integer(1))
        })
        .await
        .unwrap();
    assert_eq!(first, Some(CacheValue::Integer(1)));
    assert_eq!(second, Some(CacheValue::Integer(1)));
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cache_coordination_claim_role_hands_waiters_the_inflight_signal() {
    let coordinator = coordinator(Duration::from_secs(5));
    assert!(coordinator.claim_role().is_none());
    let mut signal = coordinator.claim_role().unwrap();
    coordinator.signal_done();
    signal.changed().await.unwrap();
    assert!(*signal.borrow());
    assert!(coordinator.claim_role().is_none());
}

#[tokio::test]
async fn cache_coordination_single_loader_serves_every_request() {
    let scenario = Scenario::new(Duration::from_secs(5));
    let mut loader = JoinSet::new();
    scenario.spawn(&mut loader, LoadKind::Blocked);
    scenario.started.notified().await;
    let mut waiter = JoinSet::new();
    scenario.spawn(&mut waiter, LoadKind::Unblocked);
    settle().await;
    scenario.release.notify_one();
    let from_load = loader.join_next().await.unwrap().unwrap().unwrap();
    let from_wait = waiter.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(from_load, Some(CacheValue::Integer(42)));
    assert_eq!(from_wait, Some(CacheValue::Integer(42)));
    assert_eq!(scenario.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_coordination_loader_failure_signals_waiters() {
    let scenario = Scenario::new(Duration::from_secs(5));
    let mut loader = JoinSet::new();
    scenario.spawn(&mut loader, LoadKind::Failing);
    scenario.started.notified().await;
    let mut waiter = JoinSet::new();
    scenario.spawn(&mut waiter, LoadKind::Unblocked);
    settle().await;
    scenario.release.notify_one();
    let failed = loader.join_next().await.unwrap().unwrap();
    let from_wait = waiter.join_next().await.unwrap().unwrap().unwrap();
    assert!(matches!(failed, Err(CoordinatorError::Load { .. })));
    assert_eq!(from_wait, None);
    assert_eq!(scenario.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_coordination_cancelled_loader_signals_waiters() {
    let scenario = Scenario::new(Duration::from_secs(5));
    let mut loader = JoinSet::new();
    let pending_request = scenario.spawn(&mut loader, LoadKind::Pending);
    scenario.started.notified().await;
    let mut waiter = JoinSet::new();
    scenario.spawn(&mut waiter, LoadKind::Unblocked);
    settle().await;
    pending_request.abort();
    let cancelled = loader.join_next().await.unwrap().unwrap_err();
    let from_wait = waiter.join_next().await.unwrap().unwrap().unwrap();
    assert!(cancelled.is_cancelled());
    assert_eq!(from_wait, None);
    assert_eq!(scenario.loads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_coordination_null_load_counts_as_a_miss() {
    let cache = FakeCache::new(false);
    let coordinator = coordinator(Duration::from_secs(5));
    let loads = AtomicUsize::new(0);
    let load = || async {
        loads.fetch_add(1, Ordering::SeqCst);
        Ok::<_, FakeError>(CacheValue::Null)
    };
    let first = coordinator.get_or_load("key", &cache, load).await.unwrap();
    assert_eq!(first, None);
    assert_eq!(cache.stored("key"), Some(CacheValue::Null));
    let second = coordinator.get_or_load("key", &cache, load).await.unwrap();
    assert_eq!(second, None);
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cache_coordination_logs_only_with_a_prefix() {
    let quiet = captured_log("").await;
    assert!(!quiet.contains("Cache miss"), "{quiet}");
    let loud = captured_log("[CACHE]").await;
    assert!(loud.contains("[CACHE] Cache miss"), "{loud}");
    assert!(loud.contains("[CACHE] Starting load"), "{loud}");
}

#[tokio::test]
async fn cache_coordination_waiter_times_out_without_a_signal() {
    let cache = FakeCache::new(false);
    let coordinator = coordinator(Duration::from_millis(20));
    assert!(coordinator.claim_role().is_none());
    let waited = coordinator
        .get_or_load("key", &cache, || async {
            Ok::<_, FakeError>(CacheValue::Integer(0))
        })
        .await;
    assert!(matches!(waited, Err(CoordinatorError::WaitTimeout { .. })));
}
