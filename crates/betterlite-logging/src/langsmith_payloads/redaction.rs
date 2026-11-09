use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_privacy::snapshot::redact_user_api_key_info;

// LangsmithLogger._redact_metadata: the helper is shallow, and requester_metadata rides nested into the run.
#[must_use]
pub fn redact_metadata(
    metadata: &IndexMap<String, Json>,
    redact_user_keys: bool,
) -> IndexMap<String, Json> {
    let Json::Object(mut redacted) =
        redact_user_api_key_info(&Json::Object(metadata.clone()), redact_user_keys)
    else {
        return metadata.clone();
    };
    if let Some(Json::Object(requester)) = redacted.get("requester_metadata") {
        let scrubbed = redact_user_api_key_info(&Json::Object(requester.clone()), redact_user_keys);
        redacted.insert("requester_metadata".to_owned(), scrubbed);
    }
    redacted
}

// LangsmithLogger._build_extra_metadata: the tracer's session keys are read from requester_metadata unless a top-level one exists.
#[must_use]
pub fn build_extra_metadata(
    metadata: &IndexMap<String, Json>,
    redact_user_keys: bool,
) -> IndexMap<String, Json> {
    let mut extra = metadata.clone();
    let nested = extra.get("requester_metadata").cloned();
    if let Some(Json::Object(requester)) = nested {
        for key in ["session_id", "thread_id", "conversation_id"] {
            if let Some(value) = requester.get(key) {
                extra.entry(key.to_owned()).or_insert_with(|| value.clone());
            }
        }
    }
    redact_metadata(&extra, redact_user_keys)
}
