use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::SpeechError;

pub struct SpeechKeys<'key> {
    pub explicit: Option<&'key SecretString>,
    pub global_api_key: Option<&'key SecretString>,
    pub global_openai_key: Option<&'key SecretString>,
    pub environment: Option<&'key SecretString>,
}

// LiteLLM chains the sources with `or`, so an empty key falls through to the next one
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    keys: &SpeechKeys<'_>,
) -> Result<IndexMap<String, String>, SpeechError> {
    let api_key = [keys.explicit, keys.global_api_key, keys.global_openai_key]
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty())
        .or(keys.environment)
        .ok_or(SpeechError::ApiKeyRequired)?;
    let mut updated = headers.clone();
    updated.insert("xi-api-key".to_owned(), api_key.expose_secret().to_owned());
    updated.insert("Content-Type".to_owned(), "application/json".to_owned());
    Ok(updated)
}
