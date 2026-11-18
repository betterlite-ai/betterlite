use std::collections::BTreeMap;

use betterlite_redis::cache_primitives::decode::CachedValue;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::model_budget_values::amount_text::amount_from_text;

// with arbitrary precision on, a bare JSON number reaches an untagged enum as a one-entry map under this key
const ARBITRARY_PRECISION_KEY: &str = "$serde_json::private::Number";

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum CounterMapError {
    #[error("a counter value has a JSON shape that holds no amount")]
    WrongShape,
    #[error("\"{text}\" does not fit a finite decimal counter")]
    NotDecimal { text: String },
}

pub(super) fn decimal_map(
    values: &BTreeMap<String, Option<CachedValue>>,
) -> Result<BTreeMap<String, Option<Decimal>>, CounterMapError> {
    let mut amounts = BTreeMap::new();
    for (key, value) in values {
        let amount = match value {
            None => None,
            Some(cached) => decimal_of(cached)?,
        };
        amounts.insert(key.clone(), amount);
    }
    Ok(amounts)
}

fn decimal_of(value: &CachedValue) -> Result<Option<Decimal>, CounterMapError> {
    match value {
        CachedValue::Null => Ok(None),
        CachedValue::Boolean(flag) => Ok(Some(Decimal::from(u8::from(*flag)))),
        CachedValue::Number(number) => decimal_text(&number.to_string()).map(Some),
        CachedValue::Text(text) => decimal_text(text).map(Some),
        CachedValue::Object(entries) => precision_text(entries).map(Some),
        CachedValue::Array(_) => Err(CounterMapError::WrongShape),
    }
}

fn precision_text(entries: &BTreeMap<String, CachedValue>) -> Result<Decimal, CounterMapError> {
    let mut only = entries.iter();
    let Some((key, CachedValue::Text(text))) = only.next() else {
        return Err(CounterMapError::WrongShape);
    };
    if only.next().is_some() || key.as_str() != ARBITRARY_PRECISION_KEY {
        return Err(CounterMapError::WrongShape);
    }
    decimal_text(text)
}

// the JSON text keeps every digit, where a float round trip would drop a cent from a long counter
fn decimal_text(text: &str) -> Result<Decimal, CounterMapError> {
    amount_from_text(text).ok_or_else(|| CounterMapError::NotDecimal {
        text: text.to_owned(),
    })
}
