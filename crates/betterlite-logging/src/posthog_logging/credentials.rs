use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use super::settings::PostHogSettings;

pub const API_KEY_FIELD: &str = "posthog_api_key";
pub const API_URL_FIELD: &str = "posthog_api_url";

#[derive(Clone, Debug)]
pub struct Credentials {
    pub api_key: SecretString,
    pub api_url: String,
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("standard_callback_dynamic_params must be an object")]
    ParamsType,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    FieldType { field: &'static str },
}

// PostHogLogger._get_credentials_for_request: a per-request key and URL win, and each falls back alone
pub fn credentials_for_request(
    settings: &PostHogSettings,
    dynamic_params: &StandardCallbackDynamicParams,
) -> Result<Credentials, CredentialError> {
    let fields = match dynamic_params {
        Json::Null => {
            return Ok(Credentials {
                api_key: settings.api_key.clone(),
                api_url: settings.api_url.clone(),
            });
        }
        Json::Object(fields) => fields,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(CredentialError::ParamsType);
        }
    };
    let api_key = text_field(fields, API_KEY_FIELD)?
        .filter(|value| !value.is_empty())
        .map_or_else(|| settings.api_key.clone(), SecretString::from);
    let api_url = text_field(fields, API_URL_FIELD)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| settings.api_url.clone());
    Ok(Credentials { api_key, api_url })
}
fn text_field(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, CredentialError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CredentialError::FieldType { field: name })
        }
    }
}
