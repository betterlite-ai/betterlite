use std::collections::BTreeMap;
use std::future::pending;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;

use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_read_batch::local::{
    DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_MAX_LAST_READS, DEFAULT_MAX_LOCAL_ENTRIES, LocalTier,
    ReadTierSettings, TierState,
};
use crate::routing_read_batch::store::{BatchRead, BatchReadFuture, BatchReadStore, ReadWarning};
use crate::routing_read_batch::{
    ReadTiers, RequestReads, RoutingReadBatch, RoutingReadKeys, RoutingReadResults,
    RoutingReadSession, UsageReadKeys,
};
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings as StoreSettings,
};
use crate::routing_value_store::{RouteKey, RouteValue};

pub(crate) struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(0.0_f64)))
    }

    pub(crate) fn set(&self, seconds: f64) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
    }

    pub(crate) fn handle(&self) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub(crate) struct MemoryStore {
    values: Mutex<BTreeMap<RouteKey, RouteValue>>,
    warning: Option<ReadWarning>,
    reads: Mutex<Vec<Vec<RouteKey>>>,
}

impl MemoryStore {
    pub(crate) fn new(
        values: BTreeMap<RouteKey, RouteValue>,
        warning: Option<ReadWarning>,
    ) -> Self {
        Self {
            values: Mutex::new(values),
            warning,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn healthy(values: BTreeMap<RouteKey, RouteValue>) -> Self {
        Self::new(values, None)
    }

    pub(crate) fn set(&self, key: &RouteKey, value: RouteValue) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), value);
    }

    pub(crate) fn reads(&self) -> Vec<Vec<RouteKey>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchReadStore for MemoryStore {
    type Value = RouteValue;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, RouteValue> {
        Box::pin(async move {
            self.reads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(keys.to_vec());
            let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
            BatchRead {
                values: keys.iter().map(|key| values.get(key).cloned()).collect(),
                warning: self.warning,
            }
        })
    }
}

pub(crate) struct HangingStore;

impl HangingStore {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl BatchReadStore for HangingStore {
    type Value = RouteValue;

    fn batch_read<'store>(
        &'store self,
        _keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, RouteValue> {
        Box::pin(async move { pending::<BatchRead<RouteValue>>().await })
    }
}

pub(crate) fn key(raw: &str) -> RouteKey {
    RouteKey::parse(raw).unwrap()
}

pub(crate) fn keys(text: &[&str]) -> Vec<RouteKey> {
    text.iter().map(|raw_key| key(raw_key)).collect()
}

pub(crate) fn counter(value: i64) -> RouteValue {
    RouteValue::Counter(value)
}

pub(crate) fn values(entries: &[(&str, i64)]) -> BTreeMap<RouteKey, RouteValue> {
    entries
        .iter()
        .map(|(raw, value)| (key(raw), counter(*value)))
        .collect()
}

pub(crate) fn tier(clock: &TestClock, ttl: Option<f64>, expiry: f64) -> LocalTier<RouteValue> {
    LocalTier {
        settings: ReadTierSettings {
            backfill_ttl_seconds: ttl.unwrap_or(DEFAULT_LOCAL_TTL_SECONDS),
            read_expiry_seconds: expiry,
            max_entries: DEFAULT_MAX_LOCAL_ENTRIES,
            max_last_reads: DEFAULT_MAX_LAST_READS,
        },
        clock: clock.handle(),
        state: Mutex::new(TierState::default()),
    }
}

pub(crate) fn tier_capped(
    clock: &TestClock,
    max_entries: usize,
    max_last_reads: usize,
) -> LocalTier<RouteValue> {
    LocalTier {
        settings: ReadTierSettings {
            backfill_ttl_seconds: DEFAULT_LOCAL_TTL_SECONDS,
            read_expiry_seconds: 10.0_f64,
            max_entries,
            max_last_reads,
        },
        clock: clock.handle(),
        state: Mutex::new(TierState::default()),
    }
}

pub(crate) fn seed(tier: &LocalTier<RouteValue>, key: &RouteKey, value: RouteValue) {
    let fetched = BTreeMap::from([(key.clone(), value)]);
    tier.apply(slice::from_ref(key), &[None], &fetched);
}

pub(crate) fn tiers(
    cooldown: LocalTier<RouteValue>,
    usage: Option<LocalTier<RouteValue>>,
) -> ReadTiers<RouteValue> {
    ReadTiers {
        cooldown: Arc::new(cooldown),
        usage: usage.map(Arc::new),
    }
}

pub(crate) fn read_keys(cooldown: Vec<RouteKey>, usage: Option<UsageReadKeys>) -> RoutingReadKeys {
    RoutingReadKeys { cooldown, usage }
}

pub(crate) fn usage(tpm: &[&str], rpm: &[&str]) -> UsageReadKeys {
    UsageReadKeys {
        tpm: keys(tpm),
        rpm: keys(rpm),
    }
}

pub(crate) async fn read_once<S>(
    batch: &mut RoutingReadBatch<RouteValue>,
    store: &S,
    tiers: &ReadTiers<RouteValue>,
    request: &mut RequestReads<RouteValue>,
    cooldown_keys: &[RouteKey],
) -> RoutingReadResults<RouteValue>
where
    S: BatchReadStore<Value = RouteValue> + Sync,
{
    let mut session = RoutingReadSession {
        store,
        tiers,
        request,
    };
    batch.read_routing_state(&mut session, cooldown_keys).await
}

pub(crate) struct BreakerTestClock(Arc<AtomicU64>);

impl BreakerTestClock {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    pub(crate) fn set_millis(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    pub(crate) fn handle(&self) -> BreakerClock {
        let source = Arc::clone(&self.0);
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)))
    }
}

pub(crate) fn real_store(
    server: &FakeRedis,
    threshold: u32,
    recovery_millis: u64,
    clock: &BreakerTestClock,
) -> RoutingStateStore {
    let settings = StoreSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: threshold,
            recovery_timeout: Duration::from_millis(recovery_millis),
            timeout_min_duration: Duration::ZERO,
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    RoutingStateStore::new(&settings, clock.handle()).unwrap()
}

pub(crate) fn seed_remote(server: &FakeRedis, key: &RouteKey, value: &RouteValue) {
    server.seed(key.as_ref(), &serde_json::to_vec(&value).unwrap(), -1);
}
