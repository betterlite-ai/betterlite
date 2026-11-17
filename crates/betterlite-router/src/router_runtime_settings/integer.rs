use betterlite_config::sections::settings::JsonValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::router_runtime_settings::error::RouterSettingsError;

// int() in Python: an integer as is, a float truncated toward zero, a string parsed in base 10
pub fn int_setting(key: &str, value: &JsonValue) -> Result<JsonValue, RouterSettingsError> {
    let integer = match value {
        JsonValue::Number(number) if number.is_i64() || number.is_u64() => Some(value.clone()),
        JsonValue::Number(number) => number
            .as_f64()
            .and_then(|float| Decimal::from_f64(float.trunc()))
            .and_then(|truncated| {
                i64::try_from(truncated)
                    .map(JsonValue::from)
                    .or_else(|_| u64::try_from(truncated).map(JsonValue::from))
                    .ok()
            }),
        JsonValue::String(text) => text.trim().parse::<i64>().ok().map(JsonValue::from),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    };
    integer.ok_or_else(|| RouterSettingsError::NotAnInteger {
        key: key.to_owned(),
    })
}
