use std::collections::BTreeMap;

use betterlite_redis::cache_primitives::decode::CachedValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum CounterValueError {
    #[error("a counter value was {kind}, not a number or null")]
    NotNumeric { kind: &'static str },
    #[error("\"{text}\" does not fit a finite decimal counter")]
    OutOfRange { text: String },
}

pub(super) fn decimal_map<'value, Values>(
    values: Values,
) -> Result<BTreeMap<String, Option<Decimal>>, CounterValueError>
where
    Values: Iterator<Item = (&'value String, &'value Option<CachedValue>)>,
{
    let mut converted = BTreeMap::new();
    for (key, value) in values {
        let amount = match value {
            None => None,
            Some(cached) => cached_counter(cached)?,
        };
        converted.insert(key.clone(), amount);
    }
    Ok(converted)
}

fn cached_counter(value: &CachedValue) -> Result<Option<Decimal>, CounterValueError> {
    match value {
        CachedValue::Null => Ok(None),
        CachedValue::Boolean(flag) => Ok(Some(Decimal::from(u8::from(*flag)))),
        CachedValue::Number(number) => number_counter(number),
        CachedValue::Text(text) => text_counter(text),
        CachedValue::Object(_) => Err(CounterValueError::NotNumeric { kind: "an object" }),
        CachedValue::Array(_) => Err(CounterValueError::NotNumeric { kind: "an array" }),
    }
}

fn number_counter(number: &serde_json::Number) -> Result<Option<Decimal>, CounterValueError> {
    if let Some(value) = number.as_i64() {
        return Ok(Some(Decimal::from(value)));
    }
    if let Some(value) = number.as_u64() {
        return Ok(Some(Decimal::from(value)));
    }
    let Some(value) = number.as_f64() else {
        return Err(CounterValueError::OutOfRange {
            text: number.to_string(),
        });
    };
    let Some(amount) = Decimal::from_f64(value) else {
        return Err(CounterValueError::OutOfRange {
            text: number.to_string(),
        });
    };
    Ok(Some(amount))
}

// litellm validates the value as a float, so a numeric string and a bool widen the same way here
fn text_counter(text: &str) -> Result<Option<Decimal>, CounterValueError> {
    let Ok(value) = text.parse::<f64>() else {
        return Err(CounterValueError::OutOfRange {
            text: text.to_owned(),
        });
    };
    let Some(amount) = Decimal::from_f64(value) else {
        return Err(CounterValueError::OutOfRange {
            text: text.to_owned(),
        });
    };
    Ok(Some(amount))
}
