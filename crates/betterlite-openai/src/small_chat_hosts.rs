pub mod maritalk;
pub mod meta_llama;
pub mod moonshot;
pub mod morph;
pub mod nadir;
pub mod nebius;
pub mod novita;
pub mod nscale;
pub mod v0;
pub mod wandb;
pub mod zai;

mod tests;

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Value;

use crate::openai_base_config::{ConfigFamily, Headers, OpenAiConfig, OpenAiSettings};

// every host here extends OpenAIGPTConfig, so ConfigFamily::Other keeps its list and no reasoning_effort
#[must_use]
pub fn openai_parent() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

// LiteLLM's inherited mapping copies a supported param; Nebius and Wandb also rename the token count
#[derive(Debug, Clone, Copy)]
pub enum CompletionTokenRename {
    Keep,
    Rename,
}

// a supported param is copied and the rest is skipped without an error, in the caller's order
pub fn map_supported(
    non_default_params: &Object,
    optional_params: &mut Object,
    supported: &[&str],
    completion_tokens: CompletionTokenRename,
) {
    for (name, value) in non_default_params {
        let renamed = matches!(completion_tokens, CompletionTokenRename::Rename)
            && name == "max_completion_tokens";
        let supported_here = supported.contains(&name.as_str());
        if renamed || supported_here {
            let target = if renamed { "max_tokens" } else { name.as_str() };
            optional_params.insert(target.to_owned(), value.clone());
        }
    }
}

#[must_use]
// ast-grep-ignore: no-python-habits - LiteLLM reads reasoning_content with `if not`, so a falsy value is the spec
pub fn json_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub fn as_f64(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
