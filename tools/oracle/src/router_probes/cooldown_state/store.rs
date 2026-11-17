use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_router::cooldown_state::health::HealthSnapshot;
use betterlite_router::cooldown_state::record::CooldownRecord;
use betterlite_router::cooldown_state::shared::{HealthReadFuture, SharedCooldowns, SharedHealth};
use betterlite_router::routing_read_batch::store::{BatchRead, BatchReadFuture};
use betterlite_router::routing_value_store::RouteKey;

pub(super) struct FakeCooldowns {
    pub(super) records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
}

impl SharedCooldowns for FakeCooldowns {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        Box::pin(async move {
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            let values = keys.iter().map(|key| records.get(key).cloned()).collect();
            BatchRead {
                values,
                warning: None,
            }
        })
    }

    fn write(&self, key: &RouteKey, record: &CooldownRecord, _ttl_seconds: f64) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), record.clone());
    }
}

pub(super) struct FakeHealth {
    pub(super) snapshot: Mutex<Option<HealthSnapshot>>,
}

impl SharedHealth for FakeHealth {
    fn read(&self) -> HealthReadFuture<'_> {
        Box::pin(async move {
            self.snapshot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }

    fn read_raw(&self) -> Option<HealthSnapshot> {
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn write(&self, snapshot: &HealthSnapshot, _ttl_seconds: i64) {
        *self.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot.clone());
    }
}
