mod enhance;
mod tests;

use std::error::Error;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

const DEFAULT_API_BASE: &str = "https://api.perplexity.ai";
const API_BASE_ENV: &str = "PERPLEXITY_API_BASE";
const API_KEY_ENVS: [&str; 2] = ["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"];

// Every Perplexity model advertises these names, and never tools, tool_choice, function_call or functions
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
    type Error: Error;

    fn supports_reasoning(&self, model: &str) -> Result<bool, Self::Error>;
    fn supports_web_search(&self, model: &str) -> Result<bool, Self::Error>;
}

// Explicit value, then environment, then default; an empty explicit value or env var counts as unset
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
pub fn with_perplexity_fields(mut reply: Value, raw_body: &[u8]) -> Value {
    if let Err(error) = enhance::apply(&mut reply, raw_body) {
        tracing::debug!(%error, "error extracting Perplexity-specific usage fields");
    }
    reply
}
