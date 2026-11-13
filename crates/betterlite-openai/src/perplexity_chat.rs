mod enhance;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

const DEFAULT_API_BASE: &str = "https://api.perplexity.ai";
const API_BASE_ENV: &str = "PERPLEXITY_API_BASE";
const API_KEY_ENVS: [&str; 2] = ["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"];

// LiteLLM advertises these names for every Perplexity model, and never tools, tool_choice,
// function_call or functions
const SUPPORTED_PARAMS: [&str; 10] = [
    "frequency_penalty",
    "max_tokens",
    "max_completion_tokens",
    "presence_penalty",
    "response_format",
    "stream",
    "temperature",
    "top_p",
    "max_retries",
    "extra_headers",
];

// The two capability answers come from the caller, so this module never reaches the catalog.
pub trait CapabilityLookup {
    type Error: std::error::Error;

    fn supports_reasoning(&self, model: &str) -> Result<bool, Self::Error>;
    fn supports_web_search(&self, model: &str) -> Result<bool, Self::Error>;
}

// The pinned config tries the explicit value, then the environment, then the default; an empty
// explicit value or env var is unset, as docs/compatibility.md says for env text
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            environment
                .lookup(API_BASE_ENV)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let mut key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    if key.is_none() {
        for name in API_KEY_ENVS {
            if let Some(value) = environment.lookup(name).filter(|value| !value.is_empty()) {
                key = Some(SecretString::from(value));
                break;
            }
        }
    }
    (base, key)
}

// A failed lookup is logged at debug and counts as false, and the other lookup still runs
#[must_use]
pub fn supported_openai_params<L>(lookup: &L, model: &str) -> Vec<&'static str>
where
    L: CapabilityLookup,
{
    let mut params = SUPPORTED_PARAMS.to_vec();
    match lookup.supports_reasoning(model) {
        Ok(true) => params.push("reasoning_effort"),
        Ok(false) => {}
        Err(error) => {
            tracing::debug!(%error, model, "error checking if the model supports reasoning");
        }
    }
    match lookup.supports_web_search(model) {
        Ok(true) => params.push("web_search_options"),
        Ok(false) => {}
        Err(error) => {
            tracing::debug!(%error, model, "error checking if the model supports web search");
        }
    }
    params
}

// The base conversion already ran; a failure here leaves the converted reply untouched
#[must_use]
pub fn with_perplexity_fields(reply: Value, raw_body: &[u8]) -> Value {
    let mut enhanced = reply.clone();
    match enhance::apply(&mut enhanced, raw_body) {
        Ok(()) => enhanced,
        Err(error) => {
            tracing::debug!(%error, "error extracting Perplexity-specific usage fields");
            reply
        }
    }
}

// Python truthiness for a JSON value, which the pinned citation and search rules branch on
pub(crate) fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}
