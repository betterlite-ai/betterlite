pub mod error;

mod tests;

use betterlite_config::sections::mistral_resources::Settings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use error::MistralIdentityError;

pub const DEFAULT_API_BASE: &str = "https://api.mistral.ai";

#[must_use]
pub fn base_url(api_base: Option<&str>, settings: &Settings) -> String {
    let resolved = api_base
        .or(settings.api_base.as_deref())
        .unwrap_or(DEFAULT_API_BASE);
    resolved
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or_else(|| resolved.trim_end_matches('/'))
        .to_owned()
}
pub fn auth_headers(
    headers: &Headers,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<Headers, MistralIdentityError> {
    let key = api_key
        .or(settings.api_key.as_ref())
        .ok_or(MistralIdentityError::MissingApiKey)?;
    let mut merged = headers.clone();
    merged.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    Ok(merged)
}
