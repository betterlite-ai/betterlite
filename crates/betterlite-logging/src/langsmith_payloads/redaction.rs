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
    if let Some(nested) = redacted.get_mut("requester_metadata") {
        *nested = redact_user_api_key_info(nested, redact_user_keys);
    }
    redacted
}

// LangsmithLogger._build_extra_metadata: the tracer's session keys are read from requester_metadata unless a top-level one exists.
#[must_use]
pub fn build_extra_metadata(
    metadata: IndexMap<String, Json>,
    redact_user_keys: bool,
) -> IndexMap<String, Json> {
    let mut extra = metadata;
    elevate_session_keys(&mut extra);
    redact_metadata(&extra, redact_user_keys)
}

fn elevate_session_keys(extra: &mut IndexMap<String, Json>) {
    let requester = nested_requester(extra);
    for key in ["session_id", "thread_id", "conversation_id"] {
        if let Some(value) = requester.get(key) {
            extra.entry(key.to_owned()).or_insert_with(|| value.clone());
        }
    }
}

fn nested_requester(metadata: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    match metadata.get("requester_metadata") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}
