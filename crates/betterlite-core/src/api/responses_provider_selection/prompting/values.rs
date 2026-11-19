use betterlite_models::controls::json_value::JsonValue;

use crate::api::responses_provider_selection::ParameterMap;

// a null or absent setting reads as unset, as LiteLLM's kwargs.get(name) does
#[must_use]
pub(super) fn present<'map>(kwargs: &'map ParameterMap, name: &str) -> Option<&'map JsonValue> {
    match kwargs.get(name) {
        Some(JsonValue::Null) | None => None,
        Some(value) => Some(value),
    }
}

#[must_use]
pub(super) fn text_value<'map>(kwargs: &'map ParameterMap, name: &str) -> Option<&'map str> {
    match kwargs.get(name) {
        Some(JsonValue::Text(text)) => Some(text),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
