use serde_json::Value;

use crate::openai_base_config::Object;

// LiteLLM's _SUPPORTED_OPTIONAL_PARAMS, metadata included because the filter keeps it until it is dropped
pub const ALLOWED_OPTIONAL_PARAMS: [&str; 21] = [
    "instructions",
    "max_output_tokens",
    "previous_response_id",
    "store",
    "reasoning",
    "stream",
    "temperature",
    "top_p",
    "text",
    "tools",
    "tool_choice",
    "max_tool_calls",
    "thinking",
    "caching",
    "expire_at",
    "context_management",
    "metadata",
    "extra_headers",
    "extra_query",
    "extra_body",
    "timeout",
];

const ADVERTISED: [&str; 2] = ["input", "model"];
const METADATA: &str = "metadata";
const PARALLEL_TOOL_CALLS: &str = "parallel_tool_calls";
const EXTRA_BODY: &str = "extra_body";

// LiteLLM advertises input and model plus every optional key but the internal metadata
#[must_use]
pub fn supported_openai_params() -> Vec<&'static str> {
    let mut supported = Vec::from(ADVERTISED);
    let optional = ALLOWED_OPTIONAL_PARAMS
        .iter()
        .copied()
        .filter(|name| *name != METADATA);
    supported.extend(optional);
    supported
}

// map_openai_params keeps the allowed keys and drops metadata and parallel_tool_calls
#[must_use]
pub fn map_openai_params(params: &Object) -> Object {
    let mut mapped: Object = params
        .iter()
        .filter(|(name, _value)| ALLOWED_OPTIONAL_PARAMS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    mapped.remove(METADATA);
    if params.contains_key(PARALLEL_TOOL_CALLS) {
        tracing::debug!(
            "Volcengine Responses API: dropping unsupported 'parallel_tool_calls' param."
        );
    }
    mapped
}

// transform_responses_api_request filters extra_body's own keys by the same list and drops it when empty
#[must_use]
pub fn sanitize_request(params: &Object) -> Object {
    let mut sanitized = map_openai_params(params);
    let Some(Value::Object(body)) = sanitized.get(EXTRA_BODY).cloned() else {
        return sanitized;
    };
    let filtered: Object = body
        .iter()
        .filter(|(name, _value)| ALLOWED_OPTIONAL_PARAMS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if filtered.is_empty() {
        sanitized.remove(EXTRA_BODY);
    } else {
        sanitized.insert(String::from(EXTRA_BODY), Value::Object(filtered));
    }
    sanitized
}
