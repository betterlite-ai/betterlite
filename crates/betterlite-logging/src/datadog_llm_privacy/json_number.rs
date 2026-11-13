use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;

// Decimal's Display is a plain JSON number, so the fallback is unreachable for a value Decimal holds
pub(super) fn number(value: Decimal) -> Json {
    match Number::from_str(&value.to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
