use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::openai_base_config::Headers;

pub const MISSING_KEY_MESSAGE: &str = "Missing Novita AI API Key - A call is being made to novita but no key is set either in the environment variables or via params";

// LiteLLM raises a ValueError with this text; Rust has no ValueError, so B184 decides the proxy's answer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "Missing Novita AI API Key - A call is being made to novita but no key is set either in the environment variables or via params"
)]
pub struct NovitaKeyError;

#[derive(Debug, Clone, Copy, Default)]
pub struct NovitaConfig;

impl NovitaConfig {
    // an absent key fails and an empty one does not, then the three headers are set on a copy
    pub fn validate_environment(
        api_key: Option<&SecretString>,
        headers: &Headers,
    ) -> Result<Headers, NovitaKeyError> {
        let Some(key) = api_key else {
            return Err(NovitaKeyError);
        };
        let mut validated = headers.clone();
        validated.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
        validated.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
        validated.insert(String::from("X-Novita-Source"), String::from("litellm"));
        Ok(validated)
    }
}
