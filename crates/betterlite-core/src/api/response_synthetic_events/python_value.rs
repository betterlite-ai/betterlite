use betterlite_models::decisions::Json;

use crate::api::responses_stream_primitives::python_text::value_text;

#[must_use]
pub fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(value) => *value,
        Json::Number(number) => number.as_f64().is_none_or(|value| value != 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub fn text_or_empty(value: Option<&Json>) -> String {
    match value {
        Some(value) if truthy(value) => value_text(value),
        _ => String::new(),
    }
}
