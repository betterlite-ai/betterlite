use std::collections::BTreeMap;

use betterlite_redis::cache_primitives::decode::CachedValue;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::model_budget_values::amount_text::amount_from_text;

// serde_json's arbitrary_precision feature hands a bare JSON float to a derived untagged enum as this map
const ARBITRARY_PRECISION_NUMBER: &str = "$serde_json::private::Number";

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
        CachedValue::Number(number) => amount_counter(&number.to_string()),
        CachedValue::Text(text) => amount_counter(text),
        CachedValue::Object(entries) => object_counter(entries),
        CachedValue::Array(_) => Err(CounterValueError::NotNumeric { kind: "an array" }),
    }
}

fn object_counter(
    entries: &BTreeMap<String, CachedValue>,
) -> Result<Option<Decimal>, CounterValueError> {
    let Some((key, value)) = entries.first_key_value() else {
        return Err(CounterValueError::NotNumeric { kind: "an object" });
    };
    match (entries.len(), key.as_str(), value) {
        (1, ARBITRARY_PRECISION_NUMBER, CachedValue::Text(text)) => amount_counter(text),
        _ => Err(CounterValueError::NotNumeric { kind: "an object" }),
    }
}

// the JSON text keeps every digit, where a float round trip would drop a cent from a long counter
fn amount_counter(text: &str) -> Result<Option<Decimal>, CounterValueError> {
    amount_from_text(text).map_or_else(
        || {
            Err(CounterValueError::OutOfRange {
                text: text.to_owned(),
            })
        },
        |amount| Ok(Some(amount)),
    )
}
