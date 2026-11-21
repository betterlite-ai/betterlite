use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::Value;

use super::error::SpeechCallError;

// the first of the names whose value is a non-empty string, as Python's `a or b or c` reads them
pub(crate) fn first_text<'object>(object: &'object Object, names: &[&str]) -> Option<&'object str> {
    names
        .iter()
        .filter_map(|name| object.get(name).and_then(Value::as_str))
        .find(|value| !value.is_empty())
}

// serde_json objects and the logging model's ordered JSON share one wire shape
pub(crate) fn json_object(object: &Object) -> Result<IndexMap<String, Json>, SpeechCallError> {
    serde_json::from_value(Value::Object(object.clone())).map_err(|source| {
        SpeechCallError::Internal {
            message: source.to_string(),
        }
    })
}
