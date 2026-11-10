use betterlite_prompts::arguments::json::Object;

use super::error::BytezConfigError;

// get_supported_openai_params lists the map entries whose alias is a rename rather than False
pub const SUPPORTED_OPENAI_PARAMS: [&str; 7] = [
    "stream",
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "n",
    "max_retries",
];

const PARAM_MAP: [(&str, &str); 7] = [
    ("stream", "stream"),
    ("max_tokens", "max_new_tokens"),
    ("max_completion_tokens", "max_new_tokens"),
    ("temperature", "temperature"),
    ("top_p", "top_p"),
    ("n", "num_return_sequences"),
    ("max_retries", "max_retries"),
];

const UNSUPPORTED_PARAMS: [&str; 18] = [
    "seed",
    "stop",
    "logit_bias",
    "logprobs",
    "frequency_penalty",
    "presence_penalty",
    "top_logprobs",
    "modalities",
    "prediction",
    "stream_options",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "extra_headers",
    "parallel_tool_calls",
    "audio",
    "web_search_options",
];

pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: Object,
    drop_params: bool,
) -> Result<Object, BytezConfigError> {
    // LiteLLM merges with `{**non_default_params, **optional_params}`, so an optional value wins in place
    let mut merged = non_default_params.clone();
    for (key, value) in optional_params {
        merged.insert(key, value);
    }
    let mut adapted = Object::new();
    for (key, value) in &merged {
        if UNSUPPORTED_PARAMS.contains(&key.as_str()) {
            if drop_params {
                continue;
            }
            return Err(BytezConfigError::UnsupportedParam { key: key.clone() });
        }
        let target = PARAM_MAP
            .iter()
            .find(|(name, _)| name == key)
            .map_or(key.as_str(), |(_, alias)| *alias);
        adapted.insert(target.to_owned(), value.clone());
    }
    Ok(adapted)
}
