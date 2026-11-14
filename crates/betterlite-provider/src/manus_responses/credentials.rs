use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::responses_provider_contract::records::{Headers, Params};

use super::error::ManusResponsesError;
use super::json;
use super::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, MANUS_API_KEY_ENV, ManusResponsesConfig,
};

// An empty environment value counts as unset, as docs/compatibility.md requires of every env read.
pub(super) fn secret_text(
    secrets: &SecretLookup,
    environment: &std::collections::BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<SecretString>, ManusResponsesError> {
    let value = secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| ManusResponsesError::Environment { name, source })?;
    Ok(value.filter(|text| !text.expose_secret().is_empty()))
}

fn resolve_api_key(
    config: &ManusResponsesConfig,
    explicit: Option<&str>,
) -> Result<SecretString, ManusResponsesError> {
    if let Some(key) = explicit.filter(|key| !key.is_empty()) {
        return Ok(SecretString::from(key.to_owned()));
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
        .ok_or(ManusResponsesError::MissingApiKey)
}

pub(super) fn validated_headers(
    config: &ManusResponsesConfig,
    headers: &Headers,
    litellm_params: Option<&Params>,
) -> Result<Headers, ManusResponsesError> {
    let explicit = match litellm_params {
        Some(params) => json::text(params, "api_key")?,
        None => None,
    };
    let key = resolve_api_key(config, explicit)?;
    let mut validated = headers.clone();
    validated.insert(String::from(API_KEY_HEADER), key.expose_secret().to_owned());
    validated.insert(
        String::from(CONTENT_TYPE_HEADER),
        String::from(JSON_CONTENT_TYPE),
    );
    Ok(validated)
}
