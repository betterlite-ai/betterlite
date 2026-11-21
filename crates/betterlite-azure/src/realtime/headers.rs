use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RealtimeError;

pub const API_KEY_HEADER: &str = "api-key";
pub const AUTHORIZATION_HEADER: &str = "Authorization";

pub fn auth_headers(
    api_key: Option<&SecretString>,
    azure_ad_token: Option<&SecretString>,
) -> Result<Headers, RealtimeError> {
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        return Ok(Headers::from([(
            String::from(API_KEY_HEADER),
            key.expose_secret().to_owned(),
        )]));
    }
    if let Some(token) = azure_ad_token.filter(|token| !token.expose_secret().is_empty()) {
        return Ok(Headers::from([(
            String::from(AUTHORIZATION_HEADER),
            format!("Bearer {}", token.expose_secret()),
        )]));
    }
    Err(RealtimeError::MissingCredentials)
}
