use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::cooldown_state::health::HealthSnapshot;
use crate::cooldown_state::record::CooldownRecord;
use crate::routing_read_batch::store::{BatchReadFuture, BatchReadStore};
use crate::routing_value_store::RouteKey;

pub type HealthReadFuture<'store> =
    Pin<Box<dyn Future<Output = Option<HealthSnapshot>> + Send + 'store>>;

// the shared tier cooldown records live in: Redis in production, a fake in the tests and the oracle probe
pub trait SharedCooldowns: Send + Sync {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord>;

    fn write(&self, key: &RouteKey, record: &CooldownRecord, ttl_seconds: f64);
}

// the shared tier the one health snapshot lives in, with the raw read its merge does before the local fallback
pub trait SharedHealth: Send + Sync {
    fn read<'store>(&'store self) -> HealthReadFuture<'store>;

    fn read_raw(&self) -> Option<HealthSnapshot>;

    fn write(&self, snapshot: &HealthSnapshot, ttl_seconds: i64);
}

// a sized handle over the shared cooldown tier, since a routing read session names its store by type
pub struct SharedHandle(pub Arc<dyn SharedCooldowns>);

impl BatchReadStore for SharedHandle {
    type Value = CooldownRecord;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        self.0.batch_read(keys)
    }
}
