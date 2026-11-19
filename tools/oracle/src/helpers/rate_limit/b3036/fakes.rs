use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock};
use betterlite_rate_limit::rate_limit_window_store::{
    CounterCache, CounterValues, RedisCacheKind, WindowStore,
};
use betterlite_rate_limit::windowed_admission::{SlotIdGenerator, WindowedAdmission};

// key collection reads only the handler window size, so every cache, clock and slot id here is a stand-in the pass never calls
pub(super) fn admission(window_size: i64) -> WindowedAdmission {
    let clock: Arc<dyn LimiterClock> = Arc::new(EpochClock);
    let fail_closed = FailClosedPolicy { enabled: false };
    WindowedAdmission {
        window_size,
        clock: Arc::clone(&clock),
        fail_closed,
        windows: Arc::new(WindowStore {
            cache: Arc::new(UnusedCache),
            rate_limiter_script: None,
            counter_read_script: None,
            request_batch: None,
            check_and_increment_lock: Mutex::new(()),
        }),
        refund_script: None,
        gauges: Arc::new(ConcurrencyGauges {
            cache: Arc::new(UnusedGauges),
            clock,
            acquire_script: None,
            release_script: None,
            count_script: None,
            release_batch: None,
            fail_closed,
            check_and_increment_lock: Mutex::new(()),
        }),
        slot_ids: Arc::new(FixedSlot),
    }
}

struct UnusedCache;

#[async_trait::async_trait]
impl CounterCache for UnusedCache {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

struct UnusedGauges;

#[async_trait::async_trait]
impl GaugeCache for UnusedGauges {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

struct EpochClock;

impl LimiterClock for EpochClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

struct FixedSlot;

impl SlotIdGenerator for FixedSlot {
    fn next_slot_id(&self) -> String {
        "unused-slot".to_owned()
    }
}
