use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpendColumnValue {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Decimal(#[serde(with = "rust_decimal::serde::float")] Decimal),
    Array(Vec<Self>),
    Object(SpendObject),
}

#[derive(Debug, thiserror::Error)]
pub enum DailySpendValueError {
    #[error("daily numeric value exceeds Decimal range")]
    NumberRange,
    #[error("daily spend value cannot be represented as JSON text")]
    Json(#[source] serde_json::Error),
    #[error("{field} overflows its column type")]
    Overflow { field: String },
}

pub(super) fn as_optional_text(
    value: Option<&SpendColumnValue>,
) -> Result<Option<String>, DailySpendValueError> {
    Ok(match value {
        None | Some(SpendColumnValue::Null) => None,
        Some(SpendColumnValue::Text(text)) => Some(text.clone()),
        Some(SpendColumnValue::Bool(flag)) => Some(if *flag { "True" } else { "False" }.to_owned()),
        Some(SpendColumnValue::Int(whole)) => Some(whole.to_string()),
        Some(SpendColumnValue::Decimal(amount)) => Some(amount.to_string()),
        Some(composite @ (SpendColumnValue::Array(_) | SpendColumnValue::Object(_))) => {
            Some(serde_json::to_string(composite).map_err(DailySpendValueError::Json)?)
        }
    })
}

pub(super) fn as_decimal(value: Option<&SpendColumnValue>) -> Decimal {
    match value {
        Some(SpendColumnValue::Bool(flag)) => Decimal::from(u8::from(*flag)),
        Some(SpendColumnValue::Int(whole)) => Decimal::from(*whole),
        Some(SpendColumnValue::Decimal(amount)) => *amount,
        None
        | Some(
            SpendColumnValue::Null
            | SpendColumnValue::Text(_)
            | SpendColumnValue::Array(_)
            | SpendColumnValue::Object(_),
        ) => Decimal::ZERO,
    }
}

pub(super) fn as_counter(
    value: Option<&SpendColumnValue>,
    field: &str,
) -> Result<i64, DailySpendValueError> {
    as_decimal(value)
        .trunc()
        .to_i64()
        .ok_or_else(|| DailySpendValueError::Overflow {
            field: field.to_owned(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, SpendColumnValue>")]
pub struct SpendObject(BTreeMap<String, SpendColumnValue>);

impl TryFrom<BTreeMap<String, SpendColumnValue>> for SpendObject {
    type Error = DailySpendValueError;
    fn try_from(fields: BTreeMap<String, SpendColumnValue>) -> Result<Self, Self::Error> {
        // serde_json preserves out-of-range numbers in this map; they must not fall through to zero.
        if fields.contains_key("$serde_json::private::Number") {
            return Err(DailySpendValueError::NumberRange);
        }
        Ok(Self(fields))
    }
}
