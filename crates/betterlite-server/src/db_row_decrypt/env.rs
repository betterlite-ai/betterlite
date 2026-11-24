use std::collections::BTreeMap;

use betterlite_config::sections::mcp_env_values::{
    MCP_STDIO_ENABLED_ENV_VAR, is_mcp_stdio_flag_key,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::warnings::Warnings;

// decrypt_value_helper's two modes: the default drops a value it cannot open, keep_original keeps it.
const DROP_UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Nothing,
};
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Original,
};

// _decrypt_and_set_db_env_variables: the MCP stdio flag never comes from the database, and one bad value does not stop the rest.
#[must_use]
pub fn decrypt_and_export_env(
    values: &JsonMap,
    keep_original: bool,
    cipher: &DbCipher,
    environment: &mut BTreeMap<String, String>,
    warnings: &Warnings,
) -> BTreeMap<String, String> {
    let mut exported = BTreeMap::new();
    for (key, value) in values {
        if is_mcp_stdio_flag_key(key) {
            if warnings.first_stdio_flag() {
                tracing::warn!(
                    "Ignoring {} stored in the database. Set it in the proxy's environment instead",
                    MCP_STDIO_ENABLED_ENV_VAR,
                );
            }
            continue;
        }
        let policy = if keep_original {
            KEEP_ORIGINAL
        } else {
            DROP_UNREADABLE
        };
        match cipher.decrypt_value(stored_value(value.clone()), key, policy) {
            Some(StoredValue::Text(text)) => {
                let plaintext = text.expose_secret().to_owned();
                environment.insert(key.clone(), plaintext.clone());
                exported.insert(key.clone(), plaintext);
            }
            // decrypt_value_helper returns a non-string unchanged; os.environ takes None as absent and rejects the rest
            None | Some(StoredValue::Other(JsonValue::Null)) => {}
            Some(StoredValue::Other(other)) => {
                tracing::error!(
                    "Error setting env variable: {key} - str expected, not {}",
                    json_type_name(&other),
                );
            }
        }
    }
    exported
}

fn stored_value(value: JsonValue) -> StoredValue<JsonValue> {
    match value {
        JsonValue::String(text) => StoredValue::Text(SecretString::from(text)),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => StoredValue::Other(other),
    }
}

fn json_type_name(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "NoneType",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(number) if number.is_f64() => "float",
        JsonValue::Number(_) => "int",
        JsonValue::String(_) => "str",
        JsonValue::Array(_) => "list",
        JsonValue::Object(_) => "dict",
    }
}
