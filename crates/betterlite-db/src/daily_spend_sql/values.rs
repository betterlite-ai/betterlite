use super::value_decode::{MAX_DAILY_VALUE_BYTES, MAX_DAILY_VALUE_DEPTH, decode_daily_value};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_with::{FromInto, serde_as};
use std::collections::BTreeMap;

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum SpendColumnValue {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Decimal(#[serde_as(serialize_as = "FromInto<JsonMoney>")] Decimal),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, thiserror::Error)]
pub enum DailySpendValueError {
    #[error("daily spend value exceeds {MAX_DAILY_VALUE_BYTES} bytes")]
    TooLarge,
    #[error("daily spend value exceeds depth {MAX_DAILY_VALUE_DEPTH}")]
    TooDeep,
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

impl TryFrom<Box<RawValue>> for SpendColumnValue {
    type Error = DailySpendValueError;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        decode_daily_value(raw.get().as_bytes())
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct JsonMoney(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

impl From<Decimal> for JsonMoney {
    fn from(mut amount: Decimal) -> Self {
        // Whole money retains LiteLLM's decimal-point JSON form without converting through f64.
        if amount.scale() == 0 {
            amount.rescale(1);
        }
        Self(amount)
    }
}
