use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use crate::cost_usage::error::CostUsageError;

// A bool counts, an out-of-range integer is a typed overflow, anything else counts as 0.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "RawTokenCount")]
pub(super) enum TokenCount {
    Value(i64),
    Overflow(Box<Number>),
}

impl Default for TokenCount {
    fn default() -> Self {
        Self::Value(0)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawTokenCount {
    Bool(bool),
    Number(Box<Number>),
    Other(Box<IgnoredAny>),
}

impl From<RawTokenCount> for TokenCount {
    fn from(raw: RawTokenCount) -> Self {
        match raw {
            RawTokenCount::Bool(value) => Self::Value(i64::from(value)),
            RawTokenCount::Number(number) => match parse_integer_counter(&number) {
                Some(value) => Self::Value(value),
                None => Self::Overflow(number),
            },
            RawTokenCount::Other(_ignored) => Self::Value(0),
        }
    }
}

impl TokenCount {
    pub(super) fn value(&self) -> Result<i64, CostUsageError> {
        match self {
            Self::Value(value) => Ok(*value),
            Self::Overflow(_number) => Err(CostUsageError::CounterOverflow),
        }
    }
}

// A decimal point or exponent in the source digits marks a float, which counts as 0.
fn parse_integer_counter(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if number.to_string().contains(['.', 'e', 'E']) {
        return Some(0);
    }
    None
}

// For a field the caller already found inside a validated mapping entry.
pub(super) fn token_count_from_raw(raw: &RawValue) -> Result<i64, CostUsageError> {
    let parsed = serde_json::from_str(raw.get())
        .unwrap_or_else(|_error| RawTokenCount::Other(Box::new(IgnoredAny)));
    TokenCount::from(parsed).value()
}

mod tests;
