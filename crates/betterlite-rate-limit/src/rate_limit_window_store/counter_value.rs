use crate::limiter_records::CacheCounterValue;

#[derive(Debug, thiserror::Error)]
pub enum CounterValueError {
    #[error("the counter text is not a whole number")]
    Text(#[source] std::num::ParseIntError),
    #[error("the counter bytes are not UTF-8")]
    Utf8(#[source] std::str::Utf8Error),
    #[error("the counter bytes are not a whole number")]
    Bytes(#[source] std::num::ParseIntError),
    #[error("the counter float {value} has no whole-number form")]
    Float { value: f64 },
}

#[must_use]
pub fn counter_as_int(value: &CacheCounterValue) -> Result<i64, CounterValueError> {
    match value {
        CacheCounterValue::Int(number) => Ok(*number),
        CacheCounterValue::Float(number) => float_as_int(*number),
        CacheCounterValue::Text(text) => {
            text.trim().parse::<i64>().map_err(CounterValueError::Text)
        }
        CacheCounterValue::Bytes(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(CounterValueError::Utf8)?;
            text.trim().parse::<i64>().map_err(CounterValueError::Bytes)
        }
    }
}

fn float_as_int(number: f64) -> Result<i64, CounterValueError> {
    let truncated = number.trunc();
    if !truncated.is_finite() {
        return Err(CounterValueError::Float { value: number });
    }
    format!("{truncated:.0}")
        .parse::<i64>()
        .map_err(|_source| CounterValueError::Float { value: number })
}

mod tests;
