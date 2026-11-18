use std::collections::BTreeMap;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::routing_state_store::connection::Connection;
use rust_decimal::Decimal;
use tokio::sync::Semaphore;

use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::router_observations::counter::Increment;
use crate::routing_state::Settings;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{Delta, DeltaStore};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{BreakerClock, RedisSettings, Settings as RedisStoreSettings};
use crate::routing_value_store::{
    Clock, RouteKey, RouteValue, RoutingValueStore, StoreError, StoreRead, StoreSettings,
};

pub(crate) struct TestClock(Arc<AtomicU64>);

impl TestClock {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    pub(crate) fn set_millis(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    pub(crate) fn value_clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)).as_secs_f64())
    }

    pub(crate) fn breaker_clock(&self) -> BreakerClock {
        let source = Arc::clone(&self.0);
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)))
    }
}

pub(crate) fn route_key(raw: &str) -> RouteKey {
    RouteKey::parse(raw).unwrap()
}

pub(crate) fn settings(batch: bool, interval: Option<Duration>) -> Settings {
    Settings {
        sync_interval: interval,
        default_sync_interval: Duration::from_secs(1),
        should_batch_redis_writes: batch,
    }
}

pub(crate) fn local_store(clock: &TestClock) -> RoutingValueStore {
    let settings = StoreSettings {
        max_size_in_memory: Some(200),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    };
    RoutingValueStore::new(settings, clock.value_clock()).unwrap()
}

pub(crate) struct FakeStore {
    values: Mutex<BTreeMap<String, Decimal>>,
    pushed: Mutex<Vec<Delta>>,
    calls: AtomicUsize,
    down: AtomicBool,
    stale: AtomicBool,
    short_reply: AtomicBool,
    stalled: AtomicBool,
    release: Semaphore,
}

impl FakeStore {
    pub(crate) fn new() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            pushed: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            down: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            short_reply: AtomicBool::new(false),
            stalled: AtomicBool::new(false),
            release: Semaphore::new(0),
        }
    }

    pub(crate) fn seed(&self, key: &str, value: Decimal) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(crate) fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    // a reply that reports the value before the increment, as a concurrent reset would
    pub(crate) fn set_stale(&self, stale: bool) {
        self.stale.store(stale, Ordering::SeqCst);
    }

    // a pipeline that applies the push but reports no values back, as a truncated reply does
    pub(crate) fn set_short_reply(&self, short: bool) {
        self.short_reply.store(short, Ordering::SeqCst);
    }

    // a store that answers no push until the test lets it, so a scheduled flush stays in flight
    pub(crate) fn set_stalled(&self, stalled: bool) {
        self.stalled.store(stalled, Ordering::SeqCst);
    }

    pub(crate) fn release(&self) {
        self.release.add_permits(1);
    }

    pub(crate) fn amount(&self, key: &str) -> Option<Decimal> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn pushed(&self) -> Vec<Delta> {
        self.pushed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl DeltaStore for FakeStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.stalled.load(Ordering::SeqCst) {
            let _permit = self.release.acquire().await;
        }
        if self.down.load(Ordering::SeqCst) {
            return Err(RoutingStateError::NonFinite);
        }
        self.pushed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(deltas);
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let mut results = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let entry = values.entry(delta.key.as_ref().to_owned()).or_default();
            let before = *entry;
            *entry = entry.saturating_add(delta.value);
            results.push(if self.stale.load(Ordering::SeqCst) {
                before
            } else {
                *entry
            });
        }
        if self.short_reply.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        Ok(results)
    }
}

pub(crate) struct Harness {
    pub(crate) clock: TestClock,
    pub(crate) store: Arc<FakeStore>,
    pub(crate) local: Arc<RoutingValueStore>,
    pub(crate) counters: Arc<RouterCounters<FakeStore>>,
}

impl Harness {
    fn build(settings: Settings, store: Option<Arc<FakeStore>>) -> Self {
        let clock = TestClock::new();
        let local = Arc::new(local_store(&clock));
        let counters = Arc::new(RouterCounters::new(
            settings,
            Arc::clone(&local),
            store.clone(),
        ));
        Self {
            clock,
            store: store.unwrap_or_else(|| Arc::new(FakeStore::new())),
            local,
            counters,
        }
    }

    pub(crate) fn with_store(settings: Settings) -> Self {
        Self::build(settings, Some(Arc::new(FakeStore::new())))
    }

    pub(crate) fn without_store(settings: Settings) -> Self {
        Self::build(settings, None)
    }

    pub(crate) fn pending_len(&self) -> usize {
        queue_len(&self.counters.state.pending)
    }

    pub(crate) fn joined_len(&self) -> usize {
        queue_len(&self.counters.state.joined)
    }

    pub(crate) fn tracked_len(&self) -> usize {
        self.counters.get_tracked_keys().len()
    }

    // wait for the flushes the sink scheduled, as shutdown does
    pub(crate) async fn drain_flushes(&self) {
        let mut flushes = mem::take(
            &mut *self
                .counters
                .flushes
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        while flushes.join_next().await.is_some() {}
    }

    pub(crate) fn local_amount(&self, key: &str) -> Option<Decimal> {
        match self.local.get(&route_key(key)).unwrap() {
            StoreRead::Value(RouteValue::Spend(value)) => Some(value),
            StoreRead::Value(RouteValue::Counter(value)) => Some(Decimal::from(value)),
            StoreRead::Value(_) | StoreRead::Missing => None,
        }
    }
}

fn queue_len(queue: &Mutex<Vec<Delta>>) -> usize {
    queue.lock().unwrap_or_else(PoisonError::into_inner).len()
}

pub(crate) fn same_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

// a callback's usage pipeline batch for one key, as B397's success callback builds it
pub(crate) fn usage_batch(key: &str, count: usize) -> Vec<Increment> {
    vec![
        Increment {
            key: key.to_owned(),
            value: 1,
            ttl: Duration::from_secs(60),
        };
        count
    ]
}

pub(crate) fn breaker(threshold: u32) -> BreakerSettings {
    BreakerSettings {
        enabled: true,
        failure_threshold: threshold,
        recovery_timeout: Duration::ZERO,
        timeout_min_duration: Duration::from_millis(5_000),
        timeout_log_interval: Duration::from_millis(1_000),
    }
}

pub(crate) fn redis_settings(
    server: &FakeRedis,
    breaker_settings: BreakerSettings,
) -> RedisStoreSettings {
    RedisStoreSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: Some("b304".to_owned()),
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: breaker_settings,
    }
}

pub(crate) fn oversized_key() -> String {
    "k".repeat(crate::routing_value_store::MAX_KEY_BYTES.saturating_add(1))
}

pub(crate) fn key_limit_error(error: &RoutingStateError) -> (usize, usize) {
    match error {
        RoutingStateError::Local {
            source: StoreError::KeyTooLong { bytes, limit },
        } => (*bytes, *limit),
        other => panic!("expected a key length error, got {other:?}"),
    }
}
