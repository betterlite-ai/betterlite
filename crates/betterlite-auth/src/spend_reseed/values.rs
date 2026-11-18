use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::model_budget_values::amount_text::amount_from_text;

use super::rows::SpendAggregateRow;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReseedCacheValue {
    Number(Decimal),
    Text(String),
    Boolean(bool),
    Unreadable,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpendValueError {
    #[error("the stored spend is {kind}, which no number conversion accepts")]
    NotNumeric { kind: &'static str },
    #[error("{text:?} does not fit a decimal spend")]
    OutOfRange { text: String },
}

// Python's float(getattr(row, "spend", 0.0) or 0.0): an absent, null or falsey field reads as zero
#[must_use]
pub fn stored_amount(spend: Option<&JsonData>) -> Result<Decimal, SpendValueError> {
    match spend {
        None | Some(JsonData::Null) => Ok(Decimal::ZERO),
        Some(value) => json_amount(value),
    }
}

// Python's float(value) on a cache hit has no `or 0.0`, so an empty or unreadable value raises
#[must_use]
pub fn cache_amount(value: &ReseedCacheValue) -> Result<Decimal, SpendValueError> {
    match value {
        ReseedCacheValue::Number(amount) => Ok(*amount),
        ReseedCacheValue::Boolean(flag) => Ok(if *flag { Decimal::ONE } else { Decimal::ZERO }),
        ReseedCacheValue::Text(text) => {
            amount_from_text(text.trim()).ok_or_else(|| SpendValueError::OutOfRange {
                text: text.to_owned(),
            })
        }
        ReseedCacheValue::Unreadable => Err(SpendValueError::NotNumeric {
            kind: "an unreadable cache value",
        }),
    }
}

#[must_use]
fn json_amount(value: &JsonData) -> Result<Decimal, SpendValueError> {
    match value {
        JsonData::Null => Ok(Decimal::ZERO),
        JsonData::Bool(flag) => Ok(if *flag { Decimal::ONE } else { Decimal::ZERO }),
        JsonData::Number(number) => {
            let text = number.to_string();
            amount_from_text(&text).ok_or(SpendValueError::OutOfRange { text })
        }
        JsonData::String(text) if text.is_empty() => Ok(Decimal::ZERO),
        JsonData::String(text) => {
            amount_from_text(text.trim()).ok_or_else(|| SpendValueError::OutOfRange {
                text: text.to_owned(),
            })
        }
        JsonData::Array(items) if items.is_empty() => Ok(Decimal::ZERO),
        JsonData::Array(_) => Err(SpendValueError::NotNumeric { kind: "a list" }),
        JsonData::Object(entries) if entries.is_empty() => Ok(Decimal::ZERO),
        JsonData::Object(_) => Err(SpendValueError::NotNumeric { kind: "an object" }),
    }
}

// the first aggregate row's _sum.spend, read from a mapping row or from a row object's attribute
#[must_use]
pub fn aggregate_amount(row: &SpendAggregateRow) -> Result<Decimal, SpendValueError> {
    let sum = match row {
        SpendAggregateRow::Mapping(fields) => fields.get("_sum"),
        SpendAggregateRow::Attribute { sum } => sum.as_ref(),
    };
    match sum {
        Some(JsonData::Object(fields)) => stored_amount(fields.get("spend")),
        _ => Ok(Decimal::ZERO),
    }
}
