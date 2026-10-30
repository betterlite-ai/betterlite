use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use crate::cost_usage::error::CostUsageError;

// A bool counts, an out-of-range integer is a typed overflow, anything else counts as 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawTokenCount")]
pub(super) struct TokenCount(pub(super) i64);

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawTokenCount {
    Bool(bool),
    Number(Box<Number>),
    Other(Box<IgnoredAny>),
}

impl TryFrom<RawTokenCount> for TokenCount {
    type Error = CostUsageError;

    fn try_from(raw: RawTokenCount) -> Result<Self, Self::Error> {
        Ok(Self(count_from_raw(raw)?))
    }
}

fn count_from_raw(raw: RawTokenCount) -> Result<i64, CostUsageError> {
    match raw {
        RawTokenCount::Bool(value) => Ok(i64::from(value)),
        RawTokenCount::Number(number) => parse_integer_counter(&number),
        RawTokenCount::Other(_ignored) => Ok(0),
    }
}

// A decimal point or exponent in the source digits marks a float, which counts as 0.
fn parse_integer_counter(number: &Number) -> Result<i64, CostUsageError> {
    if let Some(value) = number.as_i64() {
        return Ok(value);
    }
    if number.to_string().contains(['.', 'e', 'E']) {
        return Ok(0);
    }
    Err(CostUsageError::CounterOverflow)
}

// For a field the caller already found inside a validated mapping entry.
pub(super) fn token_count_from_raw(raw: &RawValue) -> Result<i64, CostUsageError> {
    let parsed = serde_json::from_str(raw.get())
        .unwrap_or_else(|_error| RawTokenCount::Other(Box::new(IgnoredAny)));
    count_from_raw(parsed)
}

mod tests;
