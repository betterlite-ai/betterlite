use std::collections::BTreeMap;
use std::future::pending;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use crate::routing_read_batch::local::{LocalTier, ReadTierSettings, TierState};
use crate::routing_read_batch::store::{BatchRead, BatchReadFuture, BatchReadStore, ReadWarning};
use crate::routing_read_batch::{
    ReadTiers, RequestReads, RoutingReadBatch, RoutingReadKeys, RoutingReadResults,
    RoutingReadSession, UsageReadKeys,
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
            backfill_ttl_seconds: ttl,
            read_expiry_seconds: expiry,
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
