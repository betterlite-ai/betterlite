use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Number;

use crate::decisions::Json;

#[must_use]
pub fn coerce_token_limit(value: Option<&Json>) -> Option<i64> {
    match value? {
        Json::Number(number) => integer_from_number(number),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn integer_from_number(number: &Number) -> Option<i64> {
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    if let Some(unsigned) = number.as_u64() {
        return i64::try_from(unsigned).ok();
    }
    let float = number.as_f64()?;
    let decimal = Decimal::from_f64(float)?;
    i64::try_from(decimal.trunc()).ok()
}
