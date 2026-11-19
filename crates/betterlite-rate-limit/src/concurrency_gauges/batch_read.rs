use super::batch_get::batch_gauge_values;
use super::cache_value_conversion::gauge_in_flight;
use super::{ConcurrencyGauges, GaugeError, gauge_times};

mod tests;

pub(super) async fn read_local_counts(
    gauges: &ConcurrencyGauges,
    gauge_keys: &[String],
) -> Result<Vec<i64>, GaugeError> {
    let values = batch_gauge_values(gauges.cache.as_ref(), gauge_keys).await;
    let times = gauge_times(gauges.clock.as_ref())?;
    let mut counts = Vec::with_capacity(values.len());
    for value in &values {
        counts.push(gauge_in_flight(value.as_ref(), &times)?);
    }
    Ok(counts)
}
