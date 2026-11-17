use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

pub mod decisions;
pub mod image_edit;
pub mod rerank;
pub mod responses;
pub mod transcription;
pub mod videos;

mod tests;

pub const API_BASE_ENV: &str = "HOSTED_VLLM_API_BASE";
pub const API_KEY_ENV: &str = "HOSTED_VLLM_API_KEY";
pub const PLACEHOLDER_KEY: &str = "fake-api-key";

// LiteLLM's `api_key or get_secret_str(...) or "fake-api-key"`: an empty explicit value falls through
#[must_use]
pub fn resolved_api_key(
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> String {
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        return key.expose_secret().to_owned();
    }
    environment
        .lookup(API_KEY_ENV)
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| String::from(PLACEHOLDER_KEY))
}

// LiteLLM's `api_base or get_secret_str("HOSTED_VLLM_API_BASE")`, with an empty value counting as unset
#[must_use]
pub fn resolved_api_base(
    api_base: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    api_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            environment
                .lookup(API_BASE_ENV)
                .filter(|base| !base.is_empty())
        })
}

// the responses, image edit and video endpoints append their path under /v1 unless the base already ends there
#[must_use]
pub fn versioned_url(base: &str, path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    let version = if trimmed.ends_with("/v1") { "" } else { "/v1" };
    format!("{trimmed}{version}{path}")
}
