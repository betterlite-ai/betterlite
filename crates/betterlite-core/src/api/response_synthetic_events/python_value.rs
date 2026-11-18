use betterlite_models::decisions::Json;

use crate::api::responses_stream_primitives::python_text::value_text;

#[must_use]
pub fn has_content(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|numeric| numeric != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub fn json_array_or_empty(value: Option<&Json>) -> &[Json] {
    match value {
        Some(Json::List(items)) => items,
        _ => &[],
    }
}

#[must_use]
pub fn text_or_empty(value: Option<&Json>) -> String {
    match value {
        Some(found) if has_content(found) => value_text(found),
        _ => String::new(),
    }
}
