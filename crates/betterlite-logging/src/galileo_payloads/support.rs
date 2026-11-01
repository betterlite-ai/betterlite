use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// galileo.py's own guard clauses (if not role, if tags) test a JSON value for empty-or-absent.
pub(super) fn has_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|float_value| float_value != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// int(record.get(key) or 0): a missing, null or zero field reads as 0.
pub(super) fn record_i64(record: &IndexMap<String, Json>, key: &str) -> i64 {
    match record.get(key) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

// record.get(key, default) for a string field, read as given without a str() fallback.
pub(super) fn record_str_or(record: &IndexMap<String, Json>, key: &str, default: &str) -> String {
    match record.get(key) {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => default.to_owned(),
    }
}
