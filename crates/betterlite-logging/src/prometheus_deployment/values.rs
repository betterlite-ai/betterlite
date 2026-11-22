use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::label_value_record::LabelDumpValue;
use rust_decimal::Decimal;

use crate::prometheus::failure::exception::LabelException;

use super::error::SchemaError;

pub(super) const MISSING_LABEL_VALUE: &str = "None";

// LiteLLM reads these maps with .get(); only an object value names anything
pub(super) fn object_field<'value>(
    fields: &'value JsonObject,
    key: &str,
) -> Option<&'value JsonObject> {
    match fields.get(key) {
        Some(Json::Object(object)) => Some(object),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

pub(super) fn push_text(
    entries: &mut Vec<(String, LabelDumpValue)>,
    name: &str,
    value: Option<&str>,
) {
    if let Some(text) = value {
        entries.push((name.to_owned(), LabelDumpValue::String(text.to_owned())));
    }
}

// the four api-key and team labels both the failure and the fallback values carry
pub(super) fn push_identity(
    entries: &mut Vec<(String, LabelDumpValue)>,
    hashed_api_key: Option<&str>,
    api_key_alias: Option<&str>,
    team: Option<&str>,
    team_alias: Option<&str>,
) {
    push_text(entries, "hashed_api_key", hashed_api_key);
    push_text(entries, "api_key_alias", api_key_alias);
    push_text(entries, "team", team);
    push_text(entries, "team_alias", team_alias);
}

// the metadata fallback for model_group: a text field whose empty string names nothing
pub(super) fn model_group_field(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()).filter(|text| !text.is_empty()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// request_tags is declared as a list; only its text items can name a tag label
pub(super) fn tags_from_json(tags: &[Json]) -> Vec<String> {
    tags.iter()
        .filter_map(|tag| match tag {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

// str(getattr(exception, "status_code", None)), so an absent status_code is the text "None"
pub(super) fn fallback_exception_status(exception: &LabelException) -> String {
    if let Some(status) = exception.status_code.as_deref() {
        status.to_owned()
    } else {
        MISSING_LABEL_VALUE.to_owned()
    }
}

pub(super) fn failure_exception_status(exception: &LabelException) -> String {
    let status = fallback_exception_status(exception);
    let missing = status == MISSING_LABEL_VALUE || status.is_empty();
    match exception.code.as_deref() {
        Some(code) if missing => code.to_owned(),
        Some(_) | None => status,
    }
}

// a header value arrives as a JSON number or a numeric string
pub(super) fn decimal_text(text: &str) -> Option<Decimal> {
    if text.contains(['e', 'E']) {
        Decimal::from_scientific(text).ok()
    } else {
        text.parse().ok()
    }
}

// prometheus_client refuses a keyword call whose names differ from the declared ones
pub(super) fn check_labels(
    metric: &'static str,
    supported: &[String],
    pairs: &[(&str, &str)],
) -> Result<(), SchemaError> {
    for (name, _value) in pairs {
        if !supported.iter().any(|label| label.as_str() == *name) {
            return Err(SchemaError::Unsupported {
                metric,
                label: (*name).to_owned(),
            });
        }
    }
    for label in supported {
        if !pairs.iter().any(|(name, _value)| *name == label.as_str()) {
            return Err(SchemaError::Missing {
                metric,
                label: label.clone(),
            });
        }
    }
    Ok(())
}
