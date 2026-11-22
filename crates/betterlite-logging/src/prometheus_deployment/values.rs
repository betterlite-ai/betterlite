use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

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

// Python's truthiness, which the flags and header values are read with
pub(super) fn truthy(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_f64().is_some_and(|amount| amount.abs() > 0.0),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
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
    if status == MISSING_LABEL_VALUE || status.is_empty() {
        if let Some(code) = exception.code.as_deref() {
            return code.to_owned();
        }
    }
    status
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
