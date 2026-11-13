use std::collections::BTreeMap;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::file_contracts::config::Headers;

use super::error::ManusFilesError;
use super::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, MANUS_API_KEY_ENV, ManusFilesConfig,
};

// An empty environment value counts as unset, as docs/compatibility.md requires of every env read.
pub(super) fn secret_text(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<SecretString>, ManusFilesError> {
    let value = secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| ManusFilesError::Environment { name, source })?;
    Ok(value.filter(|text| !text.expose_secret().is_empty()))
}

pub(super) fn resolve_api_key(
    config: &ManusFilesConfig,
    explicit: Option<&SecretString>,
) -> Result<SecretString, ManusFilesError> {
    if let Some(key) = explicit.filter(|key| !key.expose_secret().is_empty()) {
        return Ok(key.clone());
    }
    if let Some(key) = config
        .globals
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
    {
        return Ok(key.clone());
    }
    secret_text(&config.secrets, &config.environment, MANUS_API_KEY_ENV)?
        .ok_or(ManusFilesError::MissingApiKey)
}

pub(super) fn validate_environment(
    config: &ManusFilesConfig,
    headers: &mut Headers,
    explicit: Option<&SecretString>,
) -> Result<(), ManusFilesError> {
    let key = resolve_api_key(config, explicit)?;
    headers.insert(String::from(API_KEY_HEADER), key.expose_secret().to_owned());
    headers.insert(
        String::from(CONTENT_TYPE_HEADER),
        String::from(JSON_CONTENT_TYPE),
    );
    Ok(())
}
