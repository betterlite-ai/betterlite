use std::time::Duration;

use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use super::{DEFAULT_COMPLETE_URL, SYNC_CLIENT_TIMEOUT, SdkMode};
use crate::cohere_embedding::request::transform_request;
use crate::openai_base_config::Headers;
pub struct PlanInput<'call> {
    pub mode: SdkMode,
    pub model: &'call str,
    pub input: &'call [String],
    pub optional_params: &'call Object,
    pub supplied_data: Option<&'call Object>,
    pub complete_api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub headers: &'call Headers,
    pub timeout: Duration,
}
pub struct LegacyEmbeddingPlan {
    pub mode: SdkMode,
    pub model: String,
    pub url: String,
    pub headers: Headers,
    pub data: Object,
    pub timeout: Duration,
}
#[must_use]
pub fn plan(input: &PlanInput<'_>) -> LegacyEmbeddingPlan {
    let url = input.complete_api_base.filter(|base| !base.is_empty());
    let supplied = input.supplied_data.filter(|data| !data.is_empty()).cloned();
    LegacyEmbeddingPlan {
        mode: input.mode,
        model: input.model.to_owned(),
        url: url.unwrap_or(DEFAULT_COMPLETE_URL).to_owned(),
        headers: validate_environment(input.headers, input.api_key),
        // LiteLLM's `data or _transform_request(...)` builds the request for an empty dictionary too
        data: supplied
            .unwrap_or_else(|| transform_request(input.model, input.input, input.optional_params)),
        timeout: match input.mode {
            SdkMode::Sync => SYNC_CLIENT_TIMEOUT,
            SdkMode::Async => input.timeout,
        },
    }
}

// AWS-signed requests arrive with Title-Case names, so an existing header in any casing wins
#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut merged = headers.clone();
    for (name, value) in [
        ("Request-Source", "unspecified:litellm"),
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ] {
        if !merged
            .keys()
            .any(|present| present.eq_ignore_ascii_case(name))
        {
            merged.insert(name.to_owned(), value.to_owned());
        }
    }
    let bearer_key = api_key.filter(|key| !key.expose_secret().is_empty());
    if let Some(key) = bearer_key
        && !merged
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    {
        merged.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    merged
}
