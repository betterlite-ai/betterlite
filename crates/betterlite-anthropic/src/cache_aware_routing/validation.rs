use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// LiteLLM's _NATIVE_OPTIONS: the request kwargs the Anthropic messages call forwards natively.
pub(crate) const NATIVE_OPTIONS: [&str; 14] = [
    "cache_control",
    "max_tokens",
    "output_config",
    "service_tier",
    "speed",
    "stop_sequences",
    "stream",
    "system",
    "temperature",
    "thinking",
    "tool_choice",
    "tools",
    "top_k",
    "top_p",
];

pub(crate) fn native_body(
    body: &JsonObject,
    request_kwargs: &JsonObject,
    messages: Option<&Json>,
) -> Json {
    let mut merged = body.clone();
    for (key, value) in request_kwargs {
        if NATIVE_OPTIONS.contains(&key.as_str()) {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged.insert(
        "messages".to_owned(),
        messages.cloned().unwrap_or(Json::Null),
    );
    Json::Object(merged)
}

mod tests;
