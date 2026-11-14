use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::completion::error::CompletionError;

const FIXED: [(&str, &str); 3] = [
    ("accept", "application/json"),
    ("anthropic-version", "2023-06-01"),
    ("content-type", "application/json"),
];

// no OAuth handling and no environment lookup: the key arrives as the caller resolved it, and an empty key is accepted
pub fn validate_environment(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
) -> Result<HeaderMap, CompletionError> {
    let key = api_key.ok_or(CompletionError::MissingApiKey)?;
    for (name, value) in FIXED {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    let mut secret = HeaderValue::from_str(key.expose_secret())
        .map_err(|_invalid| CompletionError::InvalidApiKey)?;
    secret.set_sensitive(true);
    headers.insert(HeaderName::from_static("x-api-key"), secret);
    Ok(headers)
}
