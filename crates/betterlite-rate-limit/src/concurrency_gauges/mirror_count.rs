use crate::limiter_records::CacheCounterValue;

use super::cache::GaugeCacheValue;
use super::cache_value_conversion::whole_number;
use super::{ConcurrencyGauges, PARALLEL_SLOT_TTL};

mod tests;

pub(super) async fn mirror_released_slots(
    gauges: &ConcurrencyGauges,
    counter_keys: &[String],
    remaining: &[CacheCounterValue],
) {
    for (counter_key, value) in counter_keys.iter().zip(remaining) {
        let Ok(count) = whole_number(value) else {
            continue;
        };
        let mirrored = GaugeCacheValue::Counter(CacheCounterValue::Int(count.max(0)));
        gauges
            .cache
            .set(counter_key, mirrored, PARALLEL_SLOT_TTL, true)
            .await;
    }
}
