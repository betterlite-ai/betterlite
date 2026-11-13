use betterlite_http::url_path::encode_segments;
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

use super::API_BASE;
use super::LITELLM_VERSION;
use super::error::BytezConfigError;
use super::messages::RawMessage;

// Bytez builds its endpoint from the fixed API root and never reads the configured api_base
pub fn complete_url(model: &str) -> Result<String, BytezConfigError> {
    let encoded = encode_segments(model, "model")?;
    Ok(format!("{API_BASE}/{encoded}"))
}

pub fn validate_environment(
    mut headers: Headers,
    messages: &[RawMessage],
    api_key: Option<&SecretString>,
) -> Result<Headers, BytezConfigError> {
    if messages.is_empty() {
        return Err(BytezConfigError::MissingMessages);
    }
    let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) else {
        return Err(BytezConfigError::MissingApiKey);
    };
    headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    headers.insert(
        String::from("Authorization"),
        format!("Key {}", key.expose_secret()),
    );
    headers.insert(
        String::from("user-agent"),
        format!("litellm/{LITELLM_VERSION}"),
    );
    Ok(headers)
}
