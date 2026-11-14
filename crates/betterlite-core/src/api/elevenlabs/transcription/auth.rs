use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::TranscriptionError;

pub struct TranscriptionKeys<'key> {
    pub explicit: Option<&'key SecretString>,
    pub environment: Option<&'key SecretString>,
}

// LiteLLM chains the two sources with `or`, so an empty explicit key falls through to the environment
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    keys: &TranscriptionKeys<'_>,
) -> Result<IndexMap<String, String>, TranscriptionError> {
    let api_key = keys
        .explicit
        .filter(|key| !key.expose_secret().is_empty())
        .or(keys.environment)
        .ok_or(TranscriptionError::ApiKeyRequired)?;
    let mut updated = headers.clone();
    updated.insert("xi-api-key".to_owned(), api_key.expose_secret().to_owned());
    Ok(updated)
}
