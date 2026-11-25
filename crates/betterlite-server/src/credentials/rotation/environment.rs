use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::config::{ConfigDecodeError, decode_config_json};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;
use secrecy::SecretString;

use crate::env_value_codec::{EnvValueCodecError, encrypt_env};

#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    #[error("the stored environment_variables value does not decode")]
    Decode(#[source] ConfigDecodeError),
    #[error("the stored environment_variables value is not a JSON object")]
    NotAnObject,
}

// _env_vars_param_value: the row holds the mapping itself, or the JSON text a config write put into it.
pub fn environment_values(stored: &JsonData) -> Result<Option<JsonMap>, EnvironmentError> {
    let decoded = match stored {
        JsonData::Null => return Ok(None),
        JsonData::String(text) => {
            decode_config_json(text.as_bytes()).map_err(EnvironmentError::Decode)?
        }
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_) => {
            stored.clone()
        }
    };
    match settings_value(decoded) {
        JsonValue::Object(values) => Ok(Some(values)),
        JsonValue::Null => Ok(None),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            Err(EnvironmentError::NotAnObject)
        }
    }
}

// _encrypt_env_variables: every decrypted value sealed with the explicit key, so the row holds only ciphertext.
pub fn sealed_values(
    decrypted: &BTreeMap<String, String>,
    cipher: &DbCipher,
    new_master_key: &SecretString,
) -> Result<JsonData, EnvValueCodecError> {
    let mapping: JsonMap = decrypted
        .iter()
        .map(|(name, value)| (name.clone(), JsonValue::String(value.clone())))
        .collect();
    let sealed = encrypt_env(&mapping, cipher, Some(new_master_key))?;
    Ok(JsonData::Object(
        sealed
            .into_iter()
            .map(|(name, value)| (name, stored_value(value)))
            .collect(),
    ))
}

fn settings_value(data: JsonData) -> JsonValue {
    match data {
        JsonData::Null => JsonValue::Null,
        JsonData::Bool(flag) => JsonValue::Bool(flag),
        JsonData::Number(number) => JsonValue::Number(number),
        JsonData::String(text) => JsonValue::String(text),
        JsonData::Array(items) => JsonValue::Array(items.into_iter().map(settings_value).collect()),
        JsonData::Object(fields) => JsonValue::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, settings_value(value)))
                .collect(),
        ),
    }
}

fn stored_value(value: JsonValue) -> JsonData {
    match value {
        JsonValue::Null => JsonData::Null,
        JsonValue::Bool(flag) => JsonData::Bool(flag),
        JsonValue::Number(number) => JsonData::Number(number),
        JsonValue::String(text) => JsonData::String(text),
        JsonValue::Array(items) => JsonData::Array(items.into_iter().map(stored_value).collect()),
        JsonValue::Object(fields) => JsonData::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, stored_value(value)))
                .collect(),
        ),
    }
}
