use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::McpEncodingError;
use crate::managed_resource_rows::mcp::{McpEnvVar, McpEnvVarScope};
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

pub type ReturnedRow = BTreeMap<String, JsonValue>;

const GLOBAL_ENV_VAR_KEY: &str = "mcp_global_env_var";
const DECRYPT_POLICY: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

#[must_use]
pub fn is_global_env_var_scope(scope: Option<McpEnvVarScope>) -> bool {
    let Some(entry_scope) = scope else {
        return true;
    };
    entry_scope != McpEnvVarScope::User
}

pub fn encrypt_global_env_var_values<N, D>(
    env_vars: &mut [JsonValue],
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    for entry in env_vars {
        let JsonValue::Object(fields) = entry else {
            return Err(McpEncodingError::EnvVarNotObject);
        };
        if stored_entry_is_user_scope(fields) {
            continue;
        }
        let Some(value) = fields.get_mut("value") else {
            continue;
        };
        let stored = match value {
            JsonValue::String(text) if text.is_empty() => continue,
            JsonValue::String(text) => StoredValue::Text(SecretString::from(text.clone())),
            other => StoredValue::Other(other.clone()),
        };
        match cipher.encrypt_value(stored, None)? {
            StoredValue::Text(sealed) => {
                *value = JsonValue::String(sealed.expose_secret().to_owned());
            }
            StoredValue::Other(other) => *value = other,
        }
    }
    Ok(())
}

pub fn decrypt_global_env_var_values(env_vars: &mut [McpEnvVar], cipher: &DbCipher) {
    for entry in env_vars {
        if !is_global_env_var_scope(Some(entry.scope)) || entry.value.is_empty() {
            continue;
        }
        let stored = StoredValue::<()>::Text(SecretString::from(std::mem::take(&mut entry.value)));
        if let Some(StoredValue::Text(plaintext)) =
            cipher.decrypt_value(stored, GLOBAL_ENV_VAR_KEY, DECRYPT_POLICY)
        {
            entry.value = plaintext.expose_secret().to_owned();
        } else {
            warn_undecryptable(&entry.name);
        }
    }
}

pub fn decrypt_stored_env_var_values(env_vars: &mut [JsonValue], cipher: &DbCipher) {
    for entry in env_vars {
        let JsonValue::Object(fields) = entry else {
            continue;
        };
        if stored_entry_is_user_scope(fields) {
            continue;
        }
        let name = entry_name(fields).to_owned();
        let Some(JsonValue::String(text)) = fields.get_mut("value") else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let stored = StoredValue::<()>::Text(SecretString::from(std::mem::take(text)));
        if let Some(StoredValue::Text(plaintext)) =
            cipher.decrypt_value(stored, GLOBAL_ENV_VAR_KEY, DECRYPT_POLICY)
        {
            *text = plaintext.expose_secret().to_owned();
        } else {
            warn_undecryptable(&name);
        }
    }
}

pub fn decrypt_returned_row_env_vars(
    row: &mut ReturnedRow,
    cipher: &DbCipher,
) -> Result<(), McpEncodingError> {
    let Some(env_vars) = row.get_mut("env_vars") else {
        return Ok(());
    };
    let parsed = match env_vars {
        JsonValue::Null | JsonValue::Object(_) => return Ok(()),
        JsonValue::Array(entries) => {
            decrypt_stored_env_var_values(entries, cipher);
            return Ok(());
        }
        JsonValue::Bool(_) | JsonValue::Number(_) => return Err(McpEncodingError::EnvVarsShape),
        JsonValue::String(text) => serde_json::from_str::<JsonValue>(text).ok(),
    };
    let Some(JsonValue::Array(mut entries)) = parsed else {
        return Ok(());
    };
    decrypt_stored_env_var_values(&mut entries, cipher);
    *env_vars = JsonValue::Array(entries);
    Ok(())
}

fn stored_entry_is_user_scope(fields: &BTreeMap<String, JsonValue>) -> bool {
    if let Some(JsonValue::String(scope)) = fields.get("scope") {
        return scope.as_str() == "user";
    }
    false
}

fn entry_name(fields: &BTreeMap<String, JsonValue>) -> &str {
    match fields.get("name") {
        Some(JsonValue::String(name)) => name.as_str(),
        Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Number(_))
        | Some(JsonValue::Array(_))
        | Some(JsonValue::Object(_))
        | None => "",
    }
}

fn warn_undecryptable(name: &str) {
    tracing::warn!(
        env_var = name,
        "MCP global env var failed to decrypt (LITELLM_SALT_KEY changed?); dropping it so ciphertext is not sent upstream"
    );
}
