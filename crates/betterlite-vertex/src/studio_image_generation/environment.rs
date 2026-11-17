use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::error::StudioImageError;
use super::settings::Settings;
use crate::studio_identity::Headers;

// validate_environment: the chosen key and the JSON content type replace whatever the caller sent under those names
pub fn validate_environment(
    caller: &BTreeMap<String, String>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<Headers, StudioImageError> {
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(settings.gemini_api_key.as_ref())
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(StudioImageError::MissingApiKey)?;
    let mut headers = Headers::new();
    for (name, value) in caller {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers.insert("x-goog-api-key".to_owned(), key.clone());
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}
