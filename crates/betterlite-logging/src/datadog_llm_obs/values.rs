use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub type Object = IndexMap<String, Json>;

#[must_use]
pub fn safe_identifier(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

#[must_use]
pub fn text_field(source: &Object, key: &str, default: &str) -> String {
    match source.get(key) {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => default.to_owned(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => String::new(),
    }
}

#[must_use]
pub fn mapping_field<'source>(source: &'source Object, key: &str) -> Option<&'source Object> {
    match source.get(key) {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

#[must_use]
pub fn content_blocks(source: &Object) -> Vec<&Object> {
    match source.get("content") {
        Some(Json::List(blocks)) => blocks.iter().filter_map(as_object).collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Vec::new(),
    }
}

#[must_use]
pub const fn as_object(value: &Json) -> Option<&Object> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub const fn as_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn is_filled(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
