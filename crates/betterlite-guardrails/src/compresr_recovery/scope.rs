use betterlite_context::agentic_loop_plan::JsonMap;
use serde_json::value::RawValue;

use crate::primitives::json_value::JsonValue;

const LITELLM_PARAMS_KEY: &str = "litellm_params";
const USER_API_KEY_AUTH_KEY: &str = "user_api_key_auth";
const API_KEY_KEY: &str = "api_key";
const METADATA_KEYS: [&str; 2] = ["metadata", "litellm_metadata"];

// The hash of the virtual key the proxy authenticated; empty without per-key auth, where the call id alone keys the store
#[must_use]
pub(super) fn caller_scope(kwargs: &JsonMap) -> String {
    let params = kwargs.get(LITELLM_PARAMS_KEY).and_then(object);
    for container in [params.as_ref(), Some(kwargs)] {
        let Some(container) = container else {
            continue;
        };
        for metadata_key in METADATA_KEYS {
            let Some(metadata) = container.get(metadata_key).and_then(object) else {
                continue;
            };
            let Some(auth) = metadata.get(USER_API_KEY_AUTH_KEY).and_then(value) else {
                continue;
            };
            if let Some(key) = api_key(&auth) {
                return key.to_owned();
            }
        }
    }
    String::new()
}

// Only a JSON object counts, the way LiteLLM's dict check skips every other JSON type
fn object(raw: &RawValue) -> Option<JsonMap> {
    serde_json::from_str(raw.get()).ok()
}

fn value(raw: &RawValue) -> Option<JsonValue> {
    serde_json::from_str(raw.get()).ok()
}

// LiteLLM trusts UserAPIKeyAuth.api_key only, never the bare client-supplied user_api_key string
fn api_key(auth: &JsonValue) -> Option<&str> {
    let JsonValue::Object(fields) = auth else {
        return None;
    };
    match fields.get(API_KEY_KEY) {
        Some(JsonValue::String(key)) if !key.is_empty() => Some(key.as_str()),
        _ => None,
    }
}
