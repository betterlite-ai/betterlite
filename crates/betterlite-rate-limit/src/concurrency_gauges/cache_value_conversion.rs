use std::str::from_utf8;

use crate::limiter_records::CacheCounterValue;

use super::cache::GaugeCacheValue;
use super::{GaugeError, GaugeTimes};

mod tests;

pub(super) fn whole_number(value: &CacheCounterValue) -> Result<i64, GaugeError> {
    match value {
        CacheCounterValue::Int(number) => Ok(*number),
        CacheCounterValue::Float(number) => match num_traits::ToPrimitive::to_i64(number) {
            Some(number) => Ok(number),
            None => Err(GaugeError::NotWholeNumber {
                value: value.clone(),
            }),
        },
        CacheCounterValue::Text(text) => parse_whole(text, value),
        CacheCounterValue::Bytes(bytes) => match from_utf8(bytes) {
            Ok(text) => parse_whole(text, value),
            Err(_) => Err(GaugeError::NotWholeNumber {
                value: value.clone(),
            }),
        },
    }
}

fn parse_whole(text: &str, value: &CacheCounterValue) -> Result<i64, GaugeError> {
    match text.trim().parse::<i64>() {
        Ok(number) => Ok(number),
        Err(_) => Err(GaugeError::NotWholeNumber {
            value: value.clone(),
        }),
    }
}

pub(super) fn gauge_in_flight(
    value: Option<&GaugeCacheValue>,
    times: &GaugeTimes,
) -> Result<i64, GaugeError> {
    match value {
        None => Ok(0),
        Some(GaugeCacheValue::Registry(registry)) => {
            let mut count = 0_i64;
            for timestamp in registry.values() {
                if *timestamp >= times.cutoff {
                    count = count.saturating_add(1);
                }
            }
            Ok(count)
        }
        Some(GaugeCacheValue::Counter(counter)) => Ok(whole_number(counter)?.max(0)),
    }
}
