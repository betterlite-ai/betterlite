use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

// LiteLLM's SpendRow is Mapping[str, object], but every real caller builds it from str/int/float/bool/None literals, so this closed set is the faithful typed replacement.
#[derive(Debug, Clone, PartialEq)]
pub enum SpendColumnValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(Decimal),
    Text(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DailySpendValueError {
    #[error("{field} must be a string or null")]
    NotText { field: &'static str },
    #[error("{field} is not a finite number")]
    NonFinite { field: &'static str },
    #[error("{field} overflows its column type")]
    Overflow { field: &'static str },
}

// None becomes "" so the normalized key column can never carry a NULL.
pub(super) fn as_text(
    value: Option<&SpendColumnValue>,
    field: &'static str,
) -> Result<String, DailySpendValueError> {
    match value {
        None | Some(SpendColumnValue::Null) => Ok(String::new()),
        Some(SpendColumnValue::Text(text)) => Ok(text.clone()),
        Some(_) => Err(DailySpendValueError::NotText { field }),
    }
}

pub(super) fn as_optional_text(
    value: Option<&SpendColumnValue>,
    field: &'static str,
) -> Result<Option<String>, DailySpendValueError> {
    match value {
        None | Some(SpendColumnValue::Null) => Ok(None),
        Some(SpendColumnValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(DailySpendValueError::NotText { field }),
    }
}

// A bool or number contributes its value; every other kind contributes zero, matching Python's duck-typed _as_int/_as_float.
pub(super) fn as_decimal(
    value: Option<&SpendColumnValue>,
    field: &'static str,
) -> Result<Decimal, DailySpendValueError> {
    match value {
        Some(SpendColumnValue::Bool(flag)) => Ok(if *flag { Decimal::ONE } else { Decimal::ZERO }),
        Some(SpendColumnValue::Int(whole)) => Ok(Decimal::from(*whole)),
        Some(SpendColumnValue::Float(fraction)) => float_to_decimal(*fraction, field),
        Some(SpendColumnValue::Decimal(amount)) => Ok(*amount),
        None | Some(SpendColumnValue::Null | SpendColumnValue::Text(_)) => Ok(Decimal::ZERO),
    }
}

fn float_to_decimal(fraction: f64, field: &'static str) -> Result<Decimal, DailySpendValueError> {
    if !fraction.is_finite() {
        return Err(DailySpendValueError::NonFinite { field });
    }
    Decimal::try_from(fraction).map_err(|_source| DailySpendValueError::Overflow { field })
}

pub(super) fn as_counter(
    value: Option<&SpendColumnValue>,
    field: &'static str,
) -> Result<i64, DailySpendValueError> {
    let truncated = as_decimal(value, field)?.trunc();
    truncated
        .to_i64()
        .ok_or(DailySpendValueError::Overflow { field })
}
