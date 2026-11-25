use http::HeaderMap;
use http::header::AUTHORIZATION;
use secrecy::{ExposeSecret as _, SecretString};

use crate::bearer_credentials::get_bearer_token;

const LITELLM_API_KEY_HEADER: &str = "x-litellm-api-key";

// LiteLLM's `if litellm_api_key` is Python truthiness; a present header is used as given (docs/compatibility.md)
#[must_use]
pub fn get_litellm_virtual_key(headers: &HeaderMap) -> SecretString {
    if let Some(value) = headers.get(LITELLM_API_KEY_HEADER) {
        let text: String = value.as_bytes().iter().copied().map(char::from).collect();
        return SecretString::from(format!("Bearer {text}"));
    }
    let authorization = headers
        .get(AUTHORIZATION)
        .map_or_else(String::new, |value| {
            value.as_bytes().iter().copied().map(char::from).collect()
        });
    SecretString::from(authorization)
}

// a scheme with an empty token falls back to the original value, which keeps a bare token intact
#[must_use]
pub fn normalize_credential_value(value: &SecretString) -> SecretString {
    let Some(stripped) = get_bearer_token(value.expose_secret()) else {
        return value.clone();
    };
    if stripped.expose_secret().is_empty() {
        return value.clone();
    }
    stripped
}
