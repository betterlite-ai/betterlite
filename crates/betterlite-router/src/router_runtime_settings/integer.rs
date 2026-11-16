use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Value;

use super::error::RouterSettingsError;

// int() in Python: an integer as is, a float truncated toward zero, a string parsed in base 10
pub fn int_setting(key: &str, value: &Value) -> Result<Value, RouterSettingsError> {
    let integer = match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(value.clone()),
        Value::Number(number) => number
            .as_f64()
            .and_then(|float| Decimal::from_f64(float.trunc()))
            .and_then(|truncated| {
                i64::try_from(truncated)
                    .map(Value::from)
                    .or_else(|_| u64::try_from(truncated).map(Value::from))
                    .ok()
            }),
        Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    };
    integer.ok_or_else(|| RouterSettingsError::NotAnInteger {
        key: key.to_owned(),
    })
}
