use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::Value;

use super::error::SpeechCallError;

// the text a JSON value holds when it is a string
pub(crate) fn text<'a>(object: &'a Object, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str)
}

// the dict a JSON value holds when it is an object
pub(crate) fn object<'a>(object: &'a Object, name: &str) -> Option<&'a Object> {
    object.get(name).and_then(Value::as_object)
}

// the first of the names whose value is a non-empty string, as Python's `a or b or c` reads them
pub(crate) fn first_text<'a>(object: &'a Object, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .filter_map(|name| text(object, name))
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
