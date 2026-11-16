use secrecy::{ExposeSecret as _, SecretString};

use super::VideoHeaders;
use super::error::FalVideoError;

pub const DEFAULT_QUEUE_BASE: &str = "https://queue.fal.run";

#[derive(Debug, Clone, Default)]
pub struct FalVideoSettings {
    pub api_key: Option<SecretString>,
}

impl FalVideoSettings {
    // LiteLLM reads an empty key as absent, so the first non-empty one of the three wins
    pub fn authorize(
        &self,
        headers: &VideoHeaders,
        api_key: Option<&SecretString>,
        litellm_api_key: Option<&SecretString>,
    ) -> Result<VideoHeaders, FalVideoError> {
        let key = [api_key, litellm_api_key, self.api_key.as_ref()]
            .into_iter()
            .flatten()
            .find(|secret| !secret.expose_secret().is_empty())
            .ok_or(FalVideoError::MissingApiKey)?;
        let mut authorized = headers.clone();
        authorized.insert(
            "Authorization".to_owned(),
            format!("Key {}", key.expose_secret()),
        );
        authorized.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(authorized)
    }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    api_base
        .filter(|explicit| !explicit.is_empty())
        .unwrap_or(DEFAULT_QUEUE_BASE)
        .trim_end_matches('/')
        .to_owned()
}
