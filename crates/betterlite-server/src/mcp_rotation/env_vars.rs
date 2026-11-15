use std::collections::BTreeMap;

use betterlite_db::mcp_server_encoding::McpEncodingError;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::{
    McpStoredDecodeError, decode_mcp_stored_contract,
};
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::codec::{encrypt_stored_value, is_falsey};
use super::error::RotationCodecError;

const GLOBAL_ENV_VAR_KEY: &str = "mcp_global_env_var";
const DROP_UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

pub fn reencrypt_global_env_var_values<N, D>(
    stored: Option<&str>,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<Option<JsonValue>, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(text) = stored else {
        return Ok(None);
    };
    let value = match decode_mcp_stored_contract(text.as_bytes()) {
        Ok(value) => value,
        Err(McpStoredDecodeError::Json(_)) => return Ok(None),
        Err(error) => return Err(RotationCodecError::Blob(error)),
    };
    let value = match value {
        JsonValue::String(inner) => match decode_mcp_stored_contract(inner.as_bytes()) {
            Ok(parsed) => parsed,
            Err(McpStoredDecodeError::Json(_)) => return Ok(None),
            Err(error) => return Err(RotationCodecError::Blob(error)),
        },
        other => other,
    };
    if is_falsey(&value) {
        return Ok(None);
    }
    let JsonValue::Array(entries) = value else {
        return Err(RotationCodecError::Encoding(McpEncodingError::EnvVarsShape));
    };
    let mut rebuilt = Vec::with_capacity(entries.len());
    let mut rotated = false;
    for entry in entries {
        let JsonValue::Object(mut fields) = entry else {
            return Err(RotationCodecError::Encoding(
                McpEncodingError::EnvVarNotObject,
            ));
        };
        rotated |= rotate_entry(&mut fields, cipher, new_key)?;
        rebuilt.push(JsonValue::Object(fields));
    }
    if !rotated {
        return Ok(None);
    }
    let text =
        serde_json::to_string(&JsonValue::Array(rebuilt)).map_err(RotationCodecError::Json)?;
    Ok(Some(JsonValue::String(text)))
}

fn rotate_entry<N, D>(
    fields: &mut BTreeMap<String, JsonValue>,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<bool, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    if !is_global_scope(fields.get("scope")) {
        return Ok(false);
    }
    let name = match fields.get("name") {
        Some(JsonValue::String(name)) => name.clone(),
        _other => "None".to_owned(),
    };
    let Some(value) = fields.get_mut("value") else {
        return Ok(false);
    };
    if is_falsey(value) {
        return Ok(false);
    }
    let JsonValue::String(text) = value.clone() else {
        return Ok(true);
    };
    let stored = StoredValue::<()>::Text(SecretString::from(text));
    match cipher.decrypt_value(stored, GLOBAL_ENV_VAR_KEY, DROP_UNREADABLE) {
        Some(StoredValue::Text(plaintext)) => {
            *value = encrypt_stored_value(
                JsonValue::String(plaintext.expose_secret().to_owned()),
                cipher,
                Some(new_key),
            )?;
            Ok(true)
        }
        Some(StoredValue::Other(())) | None => {
            tracing::warn!(
                "rotate_mcp_server_credentials_master_key: could not decrypt global env var {name}, skipping"
            );
            Ok(false)
        }
    }
}

fn is_global_scope(scope: Option<&JsonValue>) -> bool {
    !matches!(scope, Some(JsonValue::String(text)) if text == "user")
}
