use std::collections::BTreeMap;

use crate::limiter_records::{CacheCounterValue, ParallelRequestGauge, RateLimitResponse};

use super::cache::GaugeCacheValue;
use super::cache_value_conversion::whole_number;
use super::status_builder::{gauge_status, over_limit_response};
use super::{ConcurrencyGauges, GaugeError, GaugeTimes, PARALLEL_SLOT_TTL, gauge_times};

mod tests;

pub(super) async fn acquire_slots_in_memory(
    gauges: &ConcurrencyGauges,
    requested: &[ParallelRequestGauge],
    slot_id: &str,
) -> Result<RateLimitResponse, GaugeError> {
    let _guard = gauges.check_and_increment_lock.lock().await;
    let times = gauge_times(gauges.clock.as_ref())?;
    let mut states = Vec::with_capacity(requested.len());
    for gauge in requested {
        let raw_value = gauges.cache.get(&gauge.counter_key, true).await;
        let (registry, in_flight) = registry_state(raw_value.as_ref(), &times)?;
        if in_flight >= gauge.limit {
            return Ok(over_limit_response(gauge, in_flight));
        }
        states.push((registry, in_flight));
    }
    let mut statuses = Vec::with_capacity(requested.len());
    for (gauge, (registry, in_flight)) in requested.iter().zip(states) {
        let value = registry.map_or_else(
            || GaugeCacheValue::Counter(CacheCounterValue::Int(in_flight.saturating_add(1))),
            |mut slots| {
                slots.insert(slot_id.to_owned(), times.now);
                GaugeCacheValue::Registry(slots)
            },
        );
        gauges
            .cache
            .set(&gauge.counter_key, value, PARALLEL_SLOT_TTL, true)
            .await;
        statuses.push(gauge_status(gauge, in_flight.saturating_add(1), "OK"));
    }
    Ok(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: None,
    })
}

type GaugeState = (Option<BTreeMap<String, f64>>, i64);

fn registry_state(
    raw_value: Option<&GaugeCacheValue>,
    times: &GaugeTimes,
) -> Result<GaugeState, GaugeError> {
    match raw_value {
        Some(GaugeCacheValue::Registry(registry)) => {
            let mut fresh = BTreeMap::new();
            for (slot, timestamp) in registry {
                if *timestamp >= times.cutoff {
                    fresh.insert(slot.clone(), *timestamp);
                }
            }
            let in_flight = i64::try_from(fresh.len()).unwrap_or(i64::MAX);
            Ok((Some(fresh), in_flight))
        }
        Some(GaugeCacheValue::Counter(counter)) => Ok((None, whole_number(counter)?.max(0))),
        None => Ok((Some(BTreeMap::new()), 0)),
    }
}
