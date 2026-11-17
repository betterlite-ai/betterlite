use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use betterlite_router::cooldown_state::health::HealthSnapshot;
use betterlite_router::cooldown_state::record::CooldownRecord;
use betterlite_router::cooldown_state::shared::{HealthReadFuture, SharedCooldowns, SharedHealth};
use betterlite_router::routing_read_batch::store::{BatchRead, BatchReadFuture};
use betterlite_router::routing_value_store::RouteKey;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct FakeCooldowns {
    pub(super) records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
}

impl SharedCooldowns for FakeCooldowns {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        Box::pin(async move {
            let records = lock(&self.records);
            let values = keys.iter().map(|key| records.get(key).cloned()).collect();
            BatchRead {
                values,
                warning: None,
            }
        })
    }

    fn write(&self, key: &RouteKey, record: &CooldownRecord, _ttl_seconds: f64) {
        lock(&self.records).insert(key.clone(), record.clone());
    }
}

pub(super) struct FakeHealth {
    pub(super) snapshot: Mutex<Option<HealthSnapshot>>,
}

impl SharedHealth for FakeHealth {
    fn read<'store>(&'store self) -> HealthReadFuture<'store> {
        Box::pin(async move { lock(&self.snapshot).clone() })
    }

    fn read_raw(&self) -> Option<HealthSnapshot> {
        lock(&self.snapshot).clone()
    }

    fn write(&self, snapshot: &HealthSnapshot, _ttl_seconds: i64) {
        *lock(&self.snapshot) = Some(snapshot.clone());
    }
}
