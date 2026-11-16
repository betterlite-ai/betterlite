use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use secrecy::SecretString;

use super::error::StudioVeoError;
use crate::studio_files::resolve_key;
use crate::studio_identity::Headers;

#[derive(Clone, Copy)]
pub struct KeySources<'key> {
    pub api_key: Option<&'key SecretString>,
    pub litellm_params_api_key: Option<&'key SecretString>,
}

// validate_environment: the chosen key and the JSON content type replace whatever the caller sent under those names
pub fn validate_environment(
    caller: &BTreeMap<String, String>,
    keys: KeySources<'_>,
    settings: &Settings,
) -> Result<Headers, StudioVeoError> {
    let key = resolve_key(
        &[
            keys.api_key,
            keys.litellm_params_api_key,
            settings.generic_api_key.as_ref(),
        ],
        settings,
    )
    .ok_or(StudioVeoError::MissingApiKey)?;
    let mut headers = Headers::new();
    for (name, value) in caller {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers.insert("x-goog-api-key".to_owned(), key);
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}
