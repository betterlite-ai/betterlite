use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{ParallelRequestGauge, RateLimitResponse, counter_values_from_reply};
use crate::rate_limit_window_store::CounterScript;

use super::batch_read::read_local_counts;
use super::cache_value_conversion::whole_number;
use super::status_builder::gauge_status;
use super::{ConcurrencyGauges, GaugeError, PARALLEL_SLOT_TTL};

mod tests;

pub(super) async fn check_read_only(
    gauges: &ConcurrencyGauges,
    requested: &[ParallelRequestGauge],
    gauge_keys: &[String],
) -> Result<RateLimitResponse, GaugeError> {
    let counts = match gauges.count_script.as_deref() {
        Some(script) => match script_counts(script, gauge_keys, requested).await {
            Ok(counts) => counts,
            Err(error) => {
                gauges
                    .fail_closed
                    .reject_if_unverifiable("parallel_count_script", &error)?;
                tracing::warn!(%error, "parallel_count_script failed, using local mirror");
                read_local_counts(gauges, gauge_keys).await?
            }
        },
        None => read_local_counts(gauges, gauge_keys).await?,
    };
    Ok(gauge_response(requested, &counts))
}

async fn script_counts(
    script: &dyn CounterScript,
    gauge_keys: &[String],
    requested: &[ParallelRequestGauge],
) -> Result<Vec<i64>, GaugeError> {
    let ttl = RedisScalar::Text(PARALLEL_SLOT_TTL.as_secs().to_string());
    let args = vec![ttl; requested.len()];
    let reply = script.run(gauge_keys, &args).await?;
    let values = counter_values_from_reply(&reply)?;
    let mut counts = Vec::with_capacity(values.len());
    for value in &values {
        counts.push(whole_number(value)?.max(0));
    }
    Ok(counts)
}

fn gauge_response(requested: &[ParallelRequestGauge], counts: &[i64]) -> RateLimitResponse {
    let mut over_limit = false;
    let mut statuses = Vec::with_capacity(requested.len());
    for (gauge, in_flight) in requested.iter().zip(counts) {
        if *in_flight >= gauge.limit {
            over_limit = true;
            statuses.push(gauge_status(gauge, *in_flight, "OVER_LIMIT"));
            continue;
        }
        statuses.push(gauge_status(gauge, *in_flight, "OK"));
    }
    RateLimitResponse {
        overall_code: if over_limit { "OVER_LIMIT" } else { "OK" }.to_owned(),
        statuses,
        reservation_windows: None,
    }
}
