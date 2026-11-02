use serde_json::{Map, Value};

use super::SANDBOX_KEY;

#[must_use]
pub fn container_id(container: &Value) -> Option<String> {
    container
        .as_object()
        .and_then(|object| object.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
}

#[must_use]
pub fn sandbox_key(kwargs: &Map<String, Value>) -> Option<String> {
    kwargs
        .get(SANDBOX_KEY)
        .and_then(Value::as_str)
        .map(String::from)
}

include!("container_extraction/tests/cases.rs");
