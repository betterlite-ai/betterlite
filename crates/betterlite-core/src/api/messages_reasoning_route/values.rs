use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// Python truthiness of a value typed object, as `if summary:` reads it
pub(super) fn carries_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(value) => *value,
        Json::Number(value) => value.as_f64().is_none_or(|value| value.abs() > 0.0_f64),
        Json::Text(value) => !value.is_empty(),
        Json::List(value) => !value.is_empty(),
        Json::Object(value) => !value.is_empty(),
    }
}

// normalize reads the model with its "" default for an absent key, and null stays the None get_model_info raises on
pub(super) const fn normalize_model(value: Option<&Json>) -> Option<&str> {
    match value {
        None => Some(""),
        Some(Json::Text(model)) => Some(model.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

pub(super) const fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text.as_str()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

pub(super) const fn object(value: Option<&Json>) -> Option<&IndexMap<String, Json>> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    }
}
