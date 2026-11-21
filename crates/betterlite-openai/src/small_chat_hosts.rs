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

use crate::edenai_chat::ReasoningLookup;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

// every host here extends OpenAIGPTConfig, so ConfigFamily::Other keeps its list
// and leaves out OpenAIConfig's reasoning_effort
#[must_use]
pub fn openai_parent() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

#[must_use]
pub fn parent_supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    openai_parent().supported_params(model, catalog)
}

// B2221's supports_reasoning arrives through this seam; a failed lookup is no capability
#[must_use]
pub fn supports_reasoning<Lookup>(lookup: &Lookup, model: &str) -> bool
where
    Lookup: ReasoningLookup,
{
    lookup.supports_reasoning(model).unwrap_or(false)
}

// the inherited OpenAIGPTConfig.map_openai_params for a host that overrides only its list:
// a param in the list is copied and the rest is skipped without an error
pub fn map_supported(
    non_default_params: &Object,
    optional_params: &mut Object,
    supported: &[&str],
) {
    for (name, value) in non_default_params {
        if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// Nebius and Wandb map the same way: max_completion_tokens becomes max_tokens, a
// supported param is copied and the rest is skipped, and the input order decides
pub fn map_with_completion_tokens(
    non_default_params: &Object,
    optional_params: &mut Object,
    supported: &[&str],
) {
    for (name, value) in non_default_params {
        if name == "max_completion_tokens" {
            optional_params.insert("max_tokens".to_owned(), value.clone());
        } else if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// Python reads a missing or zero value as false, in a JSON value of any shape
#[must_use]
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
