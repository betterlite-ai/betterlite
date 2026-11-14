use betterlite_config::sections::compatible_image_credentials::{Host, Settings};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ImageCredentialError;
use crate::api::image_contracts::records::ImageHeaders;

pub const IMAGE_GENERATION_ENDPOINT: &str = "v1/images/generations";

// AIML strips a final /v1 because the endpoint already carries one; CometAPI keeps it and appends to it
#[must_use]
pub fn complete_url(host: Host, settings: &Settings, api_base: Option<&str>) -> String {
    let (resolved, _key) = settings.resolve(host, api_base, None);
    let trimmed = resolved.trim_end_matches('/');
    let base = match host {
        Host::Aiml => trimmed.strip_suffix("/v1").unwrap_or(trimmed),
        Host::CometApi => trimmed,
    };
    format!("{base}/{IMAGE_GENERATION_ENDPOINT}")
}

// a missing final key is the ValueError LiteLLM raises, and a valid one overwrites both headers
pub fn validate_environment(
    host: Host,
    settings: &Settings,
    api_key: Option<&SecretString>,
    headers: &ImageHeaders,
) -> Result<ImageHeaders, ImageCredentialError> {
    let (_base, key) = settings.resolve(host, None, api_key);
    let Some(key) = key else {
        return Err(match host {
            Host::Aiml => ImageCredentialError::MissingAimlKey,
            Host::CometApi => ImageCredentialError::MissingCometApiKey,
        });
    };
    let mut merged = headers.clone();
    merged.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    merged.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    Ok(merged)
}
