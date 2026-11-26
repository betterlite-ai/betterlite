use betterlite_models::decisions::Json;
use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use chrono::{DateTime, Utc};

// _normalize_datetime_for_sorting: a string that ends in Z gets every Z replaced, and any other Z is the date/time separator.
#[must_use]
pub fn decode_model_info_datetime(text: &str) -> Option<DateTime<Utc>> {
    if text.ends_with('Z') || !text.contains('Z') {
        return decode_billed_at(text);
    }
    let (date, rest) = text.split_once('Z')?;
    if rest.contains('Z') {
        return None;
    }
    decode_billed_at(&format!("{date}T{rest}"))
}

// Python hands datetimes to the same normalizer, but a JSON model_info carries text; every other JSON value normalizes to None.
#[must_use]
pub fn normalize_model_info_datetime(value: &Json) -> Option<DateTime<Utc>> {
    match value {
        Json::Text(text) => decode_model_info_datetime(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
