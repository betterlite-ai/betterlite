use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::SonioxException;

pub const MISSING_KEY: &str = "Missing Soniox API key. Set the SONIOX_API_KEY environment variable or pass api_key=... to litellm.transcription().";

#[must_use]
pub fn resolve_api_key(
    explicit: Option<&str>,
    environment: Option<&SecretString>,
) -> Option<SecretString> {
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        return Some(SecretString::from(explicit.to_owned()));
    }
    environment.cloned()
}

#[must_use]
pub fn resolve_api_base(explicit: Option<&str>, environment: &str) -> String {
    let base = explicit
        .filter(|value| !value.is_empty())
        .unwrap_or(environment);
    base.trim_end_matches('/').to_owned()
}

pub fn validate_environment(
    headers: Headers,
    explicit_key: Option<&str>,
    environment_key: Option<&SecretString>,
) -> Result<Headers, SonioxException> {
    let resolved = resolve_api_key(explicit_key, environment_key);
    let Some(key) = resolved.filter(|key| !key.expose_secret().is_empty()) else {
        return Err(SonioxException {
            message: MISSING_KEY.to_owned(),
            status_code: 401,
            headers: None,
        });
    };
    let mut merged = Headers::new();
    merged.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    merged.extend(headers);
    Ok(merged)
}
