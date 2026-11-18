use std::str::from_utf8;

use crate::limiter_records::CacheCounterValue;

use super::cache::GaugeCacheValue;
use super::{GaugeError, GaugeTimes};

mod tests;

pub(super) fn whole_number(value: &CacheCounterValue) -> Result<i64, GaugeError> {
    let parsed = match value {
        CacheCounterValue::Int(number) => Some(*number),
        CacheCounterValue::Float(number) => num_traits::ToPrimitive::to_i64(number),
        CacheCounterValue::Text(text) => text.trim().parse::<i64>().ok(),
        CacheCounterValue::Bytes(bytes) => from_utf8(bytes)
            .ok()
            .and_then(|text| text.trim().parse::<i64>().ok()),
    };
    parsed.ok_or_else(|| GaugeError::NotWholeNumber {
        value: value.clone(),
    })
}

pub(super) fn gauge_in_flight(
    value: Option<&GaugeCacheValue>,
    times: &GaugeTimes,
) -> Result<i64, GaugeError> {
    match value {
        None => Ok(0),
        Some(GaugeCacheValue::Registry(registry)) => Ok(i64::try_from(
            registry
                .values()
                .filter(|timestamp| **timestamp >= times.cutoff)
                .count(),
        )
        .unwrap_or(i64::MAX)),
        Some(GaugeCacheValue::Counter(counter)) => Ok(whole_number(counter)?.max(0)),
    }
}
