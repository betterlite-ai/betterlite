use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::budget_counter::coercion::{FloatInput, IntInput};

// Python's _to_float reads a JSON scalar through float(); the model map's own lexeme is the exact amount.
pub(crate) fn float_of(value: Option<&Json>) -> Option<Decimal> {
    match value? {
        Json::Bool(flag) => Some(if *flag { Decimal::ONE } else { Decimal::ZERO }),
        Json::Number(number) => number.to_string().decimal_value(),
        Json::Text(text) => text.as_str().decimal_value(),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's _to_int truncates a float token count the way int() does and refuses a non-integral string.
pub(crate) fn int_of(value: Option<&Json>) -> Option<i64> {
    match value? {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => match number.as_i64() {
            Some(integer) => Some(integer),
            None => number.as_f64().and_then(IntInput::int_value),
        },
        Json::Text(text) => text.as_str().int_value(),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}
