use crate::limiter_records::{
    CacheCounterValue, ParallelRequestGauge, RateLimitResponse, ScriptReply,
    counter_values_from_reply,
};

use super::acquire_in_memory::acquire_slots_in_memory;
use super::batch_read::read_local_counts;
use super::cache::GaugeCacheValue;
use super::cache_value_conversion::whole_number;
use super::lua_scripts::acquire_script_args;
use super::status_builder::{gauge_status, over_limit_response};
use super::{ConcurrencyGauges, GaugeError, PARALLEL_SLOT_TTL};

mod tests;

pub(super) async fn check_acquire(
    gauges: &ConcurrencyGauges,
    requested: &[ParallelRequestGauge],
    gauge_keys: &[String],
    slot_id: &str,
) -> Result<RateLimitResponse, GaugeError> {
    let local_counts = read_local_counts(gauges, gauge_keys).await?;
    for (gauge, in_flight) in requested.iter().zip(&local_counts) {
        if *in_flight >= gauge.limit {
            return Ok(over_limit_response(gauge, *in_flight));
        }
    }
    let Some(script) = gauges.acquire_script.as_deref() else {
        return acquire_slots_in_memory(gauges, requested, slot_id).await;
    };
    match script
        .run(gauge_keys, &acquire_script_args(requested, slot_id))
        .await
    {
        Ok(reply) => acquire_from_reply(gauges, requested, &reply).await,
        Err(error) => {
            gauges
                .fail_closed
                .reject_if_unverifiable("parallel_acquire_script", &error)?;
            tracing::warn!(%error, "parallel_acquire_script failed, falling back to in-memory gauge");
            acquire_slots_in_memory(gauges, requested, slot_id).await
        }
    }
}

async fn acquire_from_reply(
    gauges: &ConcurrencyGauges,
    requested: &[ParallelRequestGauge],
    reply: &ScriptReply,
) -> Result<RateLimitResponse, GaugeError> {
    let values = counter_values_from_reply(reply)?;
    let Some((first, counts)) = values.split_first() else {
        return Err(GaugeError::MissingAcquireReply);
    };
    if whole_number(first)? == 1 {
        let index = acquire_element(&values, 1)?;
        let in_flight = acquire_element(&values, 2)?;
        let gauge = usize::try_from(index)
            .ok()
            .and_then(|candidate| candidate.checked_sub(1))
            .and_then(|candidate| requested.get(candidate))
            .ok_or(GaugeError::GaugeIndex {
                index,
                gauges: requested.len(),
            })?;
        return Ok(over_limit_response(gauge, in_flight));
    }
    let mut statuses = Vec::with_capacity(counts.len());
    for (gauge, value) in requested.iter().zip(counts) {
        let in_flight = whole_number(value)?;
        let mirrored = GaugeCacheValue::Counter(CacheCounterValue::Int(in_flight));
        gauges
            .cache
            .set(&gauge.counter_key, mirrored, PARALLEL_SLOT_TTL, true)
            .await;
        statuses.push(gauge_status(gauge, in_flight, "OK"));
    }
    Ok(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: None,
    })
}

fn acquire_element(values: &[CacheCounterValue], index: usize) -> Result<i64, GaugeError> {
    let value = values.get(index).ok_or(GaugeError::ShortAcquireReply {
        needed: index.saturating_add(1),
        length: values.len(),
    })?;
    whole_number(value)
}
