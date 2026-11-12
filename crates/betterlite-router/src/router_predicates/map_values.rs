use betterlite_models::decisions::Json;

use super::JsonMap;

pub(super) fn text_value(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn declared_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

pub(super) const fn object_value(value: &Json) -> Option<&JsonMap> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
