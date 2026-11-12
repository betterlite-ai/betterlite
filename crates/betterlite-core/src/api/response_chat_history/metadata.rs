use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::response_id_codecs::response::decode_previous_response_id;

// LiteLLM derives these from ModelResponse.model_fields plus usage, and from the Response versus ChatCompletion field difference
const CHAT_COMPLETION_FIELDS: [&str; 7] = [
    "choices",
    "created",
    "id",
    "model",
    "object",
    "system_fingerprint",
    "usage",
];
const RESPONSES_ONLY_FIELDS: [&str; 29] = [
    "background",
    "completed_at",
    "conversation",
    "created_at",
    "error",
    "incomplete_details",
    "instructions",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "output",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "reasoning",
    "safety_identifier",
    "status",
    "store",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteFinish {
    Length,
    ContentFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedOutputMetadata {
    pub metadata: Map<String, Value>,
    pub upstream_response_id: Option<String>,
    pub incomplete_reason: Option<String>,
    pub incomplete_finish: IncompleteFinish,
}

#[must_use]
pub fn shared_output_metadata(
    response_fields: Option<&Map<String, Value>>,
    response_id: Option<&str>,
    response_payload: &Value,
) -> SharedOutputMetadata {
    let incomplete_reason = incomplete_reason(response_payload);
    SharedOutputMetadata {
        metadata: provider_metadata(response_fields),
        upstream_response_id: response_id.map(decode_previous_response_id),
        incomplete_finish: finish_reason_for_incomplete(incomplete_reason.as_deref()),
        incomplete_reason,
    }
}

#[must_use]
pub fn provider_metadata(fields: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut metadata = Map::new();
    let Some(present) = fields else {
        return metadata;
    };
    for (key, value) in present {
        let chat_field = CHAT_COMPLETION_FIELDS.contains(&key.as_str());
        let responses_field = RESPONSES_ONLY_FIELDS.contains(&key.as_str());
        if value.is_null() || chat_field || responses_field {
            continue;
        }
        metadata.insert(key.clone(), value.clone());
    }
    metadata
}

#[must_use]
pub fn incomplete_reason(response_payload: &Value) -> Option<String> {
    let Value::Object(fields) = response_payload else {
        return None;
    };
    let Some(Value::Object(details)) = fields.get("incomplete_details") else {
        return None;
    };
    match details.get("reason") {
        Some(Value::String(reason)) => Some(reason.clone()),
        None | Some(_) => None,
    }
}

#[must_use]
pub fn finish_reason_for_incomplete(reason: Option<&str>) -> IncompleteFinish {
    match reason {
        Some("content_filter") => IncompleteFinish::ContentFilter,
        Some(_) | None => IncompleteFinish::Length,
    }
}
