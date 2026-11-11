use serde_json::Value;

use crate::openai_base_config::Object;

use super::error::VolcEngineChatError;

// LiteLLM's VolcEngineChatConfig.get_supported_openai_params returns this list whatever the model is
pub const SUPPORTED_OPENAI_PARAMS: [&str; 21] = [
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "max_completion_tokens",
    "max_tokens",
    "n",
    "presence_penalty",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "max_retries",
    "extra_headers",
    "thinking",
];

const THINKING_TYPES: [&str; 3] = ["enabled", "disabled", "auto"];

// the OpenAI-like base mapper filters by the provider list and renames the token limit, then thinking moves
pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &mut Object,
) -> Result<(), VolcEngineChatError> {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    if let Some(limit) = non_default_params.get("max_completion_tokens") {
        optional_params.insert(String::from("max_tokens"), limit.clone());
        optional_params.remove("max_completion_tokens");
    }
    if let Some(thinking) = optional_params.remove("thinking") {
        let legal = thinking
            .as_object()
            .and_then(|fields| fields.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| THINKING_TYPES.contains(&kind));
        if legal {
            match optional_params
                .entry(String::from("extra_body"))
                .or_insert_with(|| Value::Object(Object::new()))
            {
                Value::Object(fields) => {
                    fields.insert(String::from("thinking"), thinking);
                }
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_) => return Err(VolcEngineChatError::ExtraBodyNotObject),
            }
        }
    }
    Ok(())
}
