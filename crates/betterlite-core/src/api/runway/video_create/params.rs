use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Number;

use super::{SUPPORTED_OPENAI_PARAMS, VideoParams};

// map_openai_params: the advertised OpenAI names are translated, and every other field rides through last so it wins
#[must_use]
pub fn map_openai_params(options: &VideoParams) -> VideoParams {
    let mut mapped = VideoParams::new();
    if let Some(reference) = options.get("input_reference") {
        mapped.insert("promptImage".to_owned(), reference.clone());
    }
    if let Some(Json::Text(size)) = options.get("size")
        && size.contains('x')
    {
        mapped.insert("ratio".to_owned(), Json::Text(size.replace('x', ":")));
    }
    if let Some(seconds) = options.get("seconds")
        && let Some(duration) = duration_value(seconds)
    {
        mapped.insert("duration".to_owned(), duration);
    }
    for (name, value) in options {
        if !SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            mapped.insert(name.clone(), value.clone());
        }
    }
    mapped
}

// _duration_param: int() of a number or of float() of a text, and a failed conversion drops the duration
fn duration_value(seconds: &Json) -> Option<Json> {
    let duration = match seconds {
        Json::Number(number) => number_decimal(number)?,
        Json::Text(text) => Decimal::from_f64(text.trim().parse::<f64>().ok()?)?,
        Json::Bool(flag) => Decimal::from(u8::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => return None,
    };
    let truncated = duration.trunc();
    let text = if truncated.is_zero() {
        "0".to_owned()
    } else {
        truncated.to_string()
    };
    Number::from_str(&text).ok().map(Json::Number)
}

// json reads an integer lexeme as an int and a fractional one as a float, and int() truncates each in turn
fn number_decimal(number: &Number) -> Option<Decimal> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return Decimal::from_f64(number.as_f64()?);
    }
    parse_decimal_lexeme(&text).ok()
}
