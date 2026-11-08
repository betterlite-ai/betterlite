#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM passes Vault secret bodies through as free-form dicts: _json_object_body returns one and _get_secret_value_from_json_response walks one, so no fixed wire type fits"
)]
use secrecy::SecretString;
use serde_json::{Map, Value};

use super::error::VaultResponseError;

pub mod login;
mod tests;

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
