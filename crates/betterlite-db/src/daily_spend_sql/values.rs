use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum SpendColumnValue {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Decimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, thiserror::Error)]
pub enum DailySpendValueError {
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

impl SpendColumnValue {
    pub(super) fn number(&self) -> Decimal {
        match self {
            Self::Bool(flag) => Decimal::from(u8::from(*flag)),
            Self::Int(whole) => Decimal::from(*whole),
            Self::Decimal(amount) => *amount,
            Self::Null | Self::Text(_) | Self::Array(_) | Self::Object(_) => Decimal::ZERO,
        }
    }
}

pub(super) fn as_counter(
    value: Option<&SpendColumnValue>,
    field: &str,
) -> Result<i64, DailySpendValueError> {
    value
        .map_or(Decimal::ZERO, SpendColumnValue::number)
        .trunc()
        .to_i64()
        .ok_or_else(|| DailySpendValueError::Overflow {
            field: field.to_owned(),
        })
}

#[derive(Deserialize)]
struct ExactDecimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

impl TryFrom<Box<RawValue>> for SpendColumnValue {
    type Error = DailySpendValueError;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        // RawValue keeps JSON objects distinct from serde_json's private numeric map encoding.
        let text = raw.get();
        match text.as_bytes().first() {
            Some(b'n') => Ok(Self::Null),
            Some(b't' | b'f') => serde_json::from_str(text).map(Self::Bool),
            Some(b'"') => serde_json::from_str(text).map(Self::Text),
            Some(b'[') => serde_json::from_str(text).map(Self::Array),
            Some(b'{') => serde_json::from_str(text).map(Self::Object),
            _ => match text.parse::<i64>() {
                Ok(whole) => Ok(Self::Int(whole)),
                Err(_) => {
                    serde_json::from_str::<ExactDecimal>(text).map(|number| Self::Decimal(number.0))
                }
            },
        }
        .map_err(DailySpendValueError::Json)
    }
}
