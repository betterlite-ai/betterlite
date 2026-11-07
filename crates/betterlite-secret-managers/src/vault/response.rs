#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM passes Vault secret bodies through as free-form dicts: _json_object_body returns one and _get_secret_value_from_json_response walks one, so no fixed wire type fits"
)]
use secrecy::SecretString;
use serde_json::{Map, Value};

use super::error::VaultResponseError;

mod tests;

#[derive(Debug, Clone)]
pub struct VaultLogin {
    pub client_token: SecretString,
    pub lease_duration: u64,
}

pub fn decode_login_response(body: &[u8]) -> Result<VaultLogin, VaultResponseError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_source| VaultResponseError::InvalidJson)?;
    let auth = value
        .get("auth")
        .and_then(Value::as_object)
        .ok_or(VaultResponseError::MissingAuth)?;
    let client_token = auth
        .get("client_token")
        .and_then(Value::as_str)
        .ok_or(VaultResponseError::MissingToken)?;
    let lease_duration = auth
        .get("lease_duration")
        .and_then(Value::as_u64)
        .ok_or(VaultResponseError::MissingLease)?;
    Ok(VaultLogin {
        client_token: SecretString::from(client_token),
        lease_duration,
    })
}

pub fn json_object_body(body: &[u8]) -> Result<Map<String, Value>, VaultResponseError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_source| VaultResponseError::InvalidJson)?;
    match value {
        Value::Object(object) => Ok(object),
        Value::Array(_) | Value::Bool(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            Err(VaultResponseError::NotAnObject)
        }
    }
}

pub fn secret_value(body: &[u8], data_key: &str) -> Option<SecretString> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let outer = value.get("data")?.as_object()?;
    let inner = outer.get("data")?.as_object()?;
    inner.get(data_key)?.as_str().map(SecretString::from)
}
