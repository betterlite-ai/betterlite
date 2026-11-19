use crate::limiter_records::CacheCounterValue;

use super::cache::GaugeCacheValue;
use super::cache_value_conversion::whole_number;
use super::{ConcurrencyGauges, PARALLEL_SLOT_TTL};

mod tests;

pub(super) async fn release_slots_in_memory(
    gauges: &ConcurrencyGauges,
    counter_keys: &[String],
    slot_id: &str,
) {
    let _guard = gauges.check_and_increment_lock.lock().await;
    for counter_key in counter_keys {
        let Some(value) = gauges.cache.get(counter_key, true).await else {
            continue;
        };
        let released = match value {
            GaugeCacheValue::Registry(mut registry) => {
                if registry.remove(slot_id).is_none() {
                    continue;
                }
                GaugeCacheValue::Registry(registry)
            }
            GaugeCacheValue::Counter(counter) => match whole_number(&counter) {
                Ok(count) => {
                    GaugeCacheValue::Counter(CacheCounterValue::Int(count.saturating_sub(1).max(0)))
                }
                Err(_) => continue,
            },
        };
        gauges
            .cache
            .set(counter_key, released, PARALLEL_SLOT_TTL, true)
            .await;
    }
}
