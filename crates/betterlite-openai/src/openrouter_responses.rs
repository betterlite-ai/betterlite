mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::openai_base_config::Headers;

const DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";

// LiteLLM's OpenRouterResponsesAPIConfig overrides only the provider, the key and the URL
pub const PROVIDER: ProviderId = ProviderId::Openrouter;
// supports_native_websocket returns False for OpenRouter, so the OpenAI transport stays the one used
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResponsesConfigError {
    #[error(
        "OpenRouter API key is required. Set OPENROUTER_API_KEY environment variable or pass api_key parameter."
    )]
    MissingApiKey,
}

pub fn validate_environment(
    headers: Headers,
    api_key: Option<&SecretString>,
    global_api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Result<Headers, ResponsesConfigError> {
    let key = first_secret(
        [api_key, global_api_key],
        ["OPENROUTER_API_KEY", "OR_API_KEY"],
        environment,
    )
    .ok_or(ResponsesConfigError::MissingApiKey)?;
    let mut headers = headers;
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    Ok(headers)
}

#[must_use]
pub fn get_complete_url(
    api_base: Option<&str>,
    global_api_base: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> String {
    let base = first_key(
        [api_base, global_api_base],
        ["OPENROUTER_API_BASE"],
        environment,
    )
    .unwrap_or_else(|| String::from(DEFAULT_API_BASE));
    format!("{}/responses", base.trim_end_matches('/'))
}

fn first_secret<const A: usize, const N: usize>(
    arguments: [Option<&SecretString>; A],
    names: [&str; N],
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    arguments
        .into_iter()
        .flatten()
        .cloned()
        .chain(
            names
                .into_iter()
                .filter_map(|name| environment.lookup(name))
                .map(SecretString::from),
        )
        .find(|value| !value.expose_secret().is_empty())
}

// api_base is not a secret, so it stays a plain string
fn first_key<const A: usize, const N: usize>(
    arguments: [Option<&str>; A],
    names: [&str; N],
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    arguments
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .chain(
            names
                .into_iter()
                .filter_map(|name| environment.lookup(name)),
        )
        .find(|value| !value.is_empty())
}
