use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::routing_state_store::connection::Connection;
use rust_decimal::Decimal;

use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_state::Settings;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{Delta, DeltaStore};
use crate::routing_state::error::RoutingStateError;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings as RedisStoreSettings,
};
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
    calls: AtomicUsize,
    down: AtomicBool,
    stale: AtomicBool,
}

impl FakeStore {
    pub(crate) fn new() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            calls: AtomicUsize::new(0),
            down: AtomicBool::new(false),
            stale: AtomicBool::new(false),
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
}

#[async_trait]
impl DeltaStore for FakeStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.down.load(Ordering::SeqCst) {
            return Err(RoutingStateError::NonFinite);
        }
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let mut results = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let entry = values.entry(delta.key.as_ref().to_owned()).or_default();
            let before = *entry;
            *entry = entry.saturating_add(Decimal::from(delta.value));
            results.push(if self.stale.load(Ordering::SeqCst) {
                before
            } else {
                *entry
            });
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
    pub(crate) fn with_store(settings: Settings) -> Self {
        let clock = TestClock::new();
        let store = Arc::new(FakeStore::new());
        let local = Arc::new(local_store(&clock));
        let counters = Arc::new(RouterCounters::new(
            settings,
            Arc::clone(&local),
            Some(Arc::clone(&store)),
        ));
        Self {
            clock,
            store,
            local,
            counters,
        }
    }

    pub(crate) fn without_store(settings: Settings) -> Self {
        let clock = TestClock::new();
        let local = Arc::new(local_store(&clock));
        let counters = Arc::new(RouterCounters::new(settings, Arc::clone(&local), None));
        Self {
            clock,
            store: Arc::new(FakeStore::new()),
            local,
            counters,
        }
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.counters
            .state
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn tracked_len(&self) -> usize {
        self.counters
            .state
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn local_amount(&self, key: &str) -> Option<Decimal> {
        match self.local.get(&route_key(key)).unwrap() {
            StoreRead::Value(RouteValue::Spend(value)) => Some(value),
            StoreRead::Value(RouteValue::Counter(value)) => Some(Decimal::from(value)),
            StoreRead::Value(_) | StoreRead::Missing => None,
        }
    }
}

pub(crate) fn same_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
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
