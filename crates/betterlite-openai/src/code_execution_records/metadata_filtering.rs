use serde_json::{Map, Value};

use super::{
    CONVERTED_STREAM_KEY, INTERCEPTION_ACTIVE_KEY, INTERCEPTION_INTERNAL_PREFIXES, SANDBOX_KEY,
    SESSION_SCOPED_KEY,
};

pub fn strip(kwargs: &mut Map<String, Value>) {
    let Some(Value::Object(metadata)) = kwargs.get_mut("litellm_metadata") else {
        return;
    };
    metadata.retain(|key, _| {
        !INTERCEPTION_INTERNAL_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
            && !key.starts_with("_agentic_loop")
            && key.as_str() != "max_agentic_loops"
    });
    if metadata.is_empty() {
        kwargs.remove("litellm_metadata");
    }
}

pub fn write(kwargs: &mut Map<String, Value>) {
    let mut metadata = kwargs
        .get("litellm_metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        INTERCEPTION_ACTIVE_KEY,
        SANDBOX_KEY,
        SESSION_SCOPED_KEY,
        CONVERTED_STREAM_KEY,
    ] {
        if let Some(value) = kwargs.get(key) {
            metadata.insert(key.to_owned(), value.clone());
        }
    }
    kwargs.insert("litellm_metadata".to_owned(), Value::Object(metadata));
}

#[must_use]
pub fn sandbox_key(metadata: &Map<String, Value>) -> Option<String> {
    metadata
        .get("sandbox_key")
        .and_then(Value::as_str)
        .map(String::from)
}

include!("metadata_filtering/tests/cases.rs");
