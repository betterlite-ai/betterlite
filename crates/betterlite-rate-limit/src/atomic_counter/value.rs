use std::str::from_utf8;

use crate::limiter_records::CacheCounterValue;

mod tests;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CounterValueError {
    #[error("the rate limit value at {position} is not an integer: {value:?}")]
    NotAnInteger {
        position: String,
        value: CacheCounterValue,
    },
}

pub fn counter_integer(
    position: &str,
    value: &CacheCounterValue,
) -> Result<i64, CounterValueError> {
    match value {
        CacheCounterValue::Int(number) => Ok(*number),
        CacheCounterValue::Float(_) => Err(not_an_integer(position, value)),
        CacheCounterValue::Text(text) => text
            .parse::<i64>()
            .map_err(|_source| not_an_integer(position, value)),
        CacheCounterValue::Bytes(bytes) => from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| not_an_integer(position, value)),
    }
}

fn not_an_integer(position: &str, value: &CacheCounterValue) -> CounterValueError {
    CounterValueError::NotAnInteger {
        position: position.to_owned(),
        value: value.clone(),
    }
}
