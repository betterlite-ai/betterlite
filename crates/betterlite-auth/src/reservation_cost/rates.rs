use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::budget_counter::coercion::{FloatInput as _, IntInput};

// Python's _to_float and _to_int read the same JSON scalars and differ only in the target type.
enum Scalar<'json> {
    Bool(bool),
    Number(&'json Number),
    Text(&'json str),
}

fn scalar(value: Option<&Json>) -> Option<Scalar<'_>> {
    match value? {
        Json::Bool(flag) => Some(Scalar::Bool(*flag)),
        Json::Number(number) => Some(Scalar::Number(number)),
        Json::Text(text) => Some(Scalar::Text(text.as_str())),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's _to_float reads a JSON scalar through float(); the model map's own lexeme is the exact amount.
pub(crate) fn float_of(value: Option<&Json>) -> Option<Decimal> {
    match scalar(value)? {
        Scalar::Bool(flag) => Some(if flag { Decimal::ONE } else { Decimal::ZERO }),
        Scalar::Number(number) => number.to_string().decimal_value(),
        Scalar::Text(text) => text.decimal_value(),
    }
}

// Python's _to_int truncates a float token count the way int() does and refuses a non-integral string.
pub(crate) fn int_of(value: Option<&Json>) -> Option<i64> {
    match scalar(value)? {
        Scalar::Bool(flag) => Some(i64::from(flag)),
        Scalar::Number(number) => number
            .as_i64()
            .map_or_else(|| number.as_f64().and_then(IntInput::int_value), Some),
        Scalar::Text(text) => text.int_value(),
    }
}
