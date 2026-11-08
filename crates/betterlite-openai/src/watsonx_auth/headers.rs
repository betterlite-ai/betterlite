use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::environment::WatsonxEnvironment;
use super::error::WatsonxError;
use super::types::WatsonxParams;

pub(super) fn token_from_params(
    params: &WatsonxParams,
    environment: &WatsonxEnvironment,
) -> Result<Option<SecretString>, WatsonxError> {
    let requested_token = match params.get("token") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => Some(SecretString::from(text.clone())),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(WatsonxError::InvalidField { field: "token" });
        }
    };
    let environment_token = environment
        .watsonx_token
        .as_ref()
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .cloned();
    Ok(requested_token.or(environment_token))
}

pub(super) fn zen_api_key_from_params(
    params: &mut WatsonxParams,
    environment: &WatsonxEnvironment,
) -> Result<Option<SecretString>, WatsonxError> {
    let requested_zen = match params.remove("zen_api_key") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => Some(SecretString::from(text)),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(WatsonxError::InvalidField {
                field: "zen_api_key",
            });
        }
    };
    let environment_zen = environment
        .watsonx_zenapikey
        .as_ref()
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .cloned();
    Ok(requested_zen.or(environment_zen))
}
