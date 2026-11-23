use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

pub mod for_db;

mod tests;

// decrypt_value_helper's default "error" mode with return_original_value=True: only the debug line, the stored value stays.
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Original,
};

#[derive(Debug, thiserror::Error)]
pub enum EnvValueCodecError {
    // the key names the map entry; the value never reaches the error text
    #[error("unable to encrypt the value for key: {key}")]
    Encrypt {
        key: String,
        #[source]
        source: SecretError,
    },
}

pub fn encrypt_env<N, D>(
    values: &JsonMap,
    cipher: &StoredCipher<N, D>,
    new_key: Option<&SecretString>,
) -> Result<JsonMap, EnvValueCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let mut encrypted = JsonMap::new();
    for (key, value) in values {
        encrypted.insert(
            key.clone(),
            encrypt_entry(key, value.clone(), cipher, new_key)?,
        );
    }
    Ok(encrypted)
}

#[must_use]
pub fn decrypt_env<N, D>(values: &JsonMap, cipher: &StoredCipher<N, D>) -> JsonMap
where
    D: SecretDiagnostics,
{
    values
        .iter()
        .map(|(key, value)| (key.clone(), decrypt_entry(key, value.clone(), cipher)))
        .collect()
}

fn encrypt_entry<N, D>(
    key: &str,
    value: JsonValue,
    cipher: &StoredCipher<N, D>,
    new_key: Option<&SecretString>,
) -> Result<JsonValue, EnvValueCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    cipher
        .encrypt_value(stored_value(value), new_key)
        .map(plain_value)
        .map_err(|source| EnvValueCodecError::Encrypt {
            key: key.to_owned(),
            source,
        })
}

fn decrypt_entry<N, D>(key: &str, value: JsonValue, cipher: &StoredCipher<N, D>) -> JsonValue
where
    D: SecretDiagnostics,
{
    let stored = stored_value(value.clone());
    cipher
        .decrypt_value(stored, key, KEEP_ORIGINAL)
        .map_or(value, plain_value)
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

fn plain_value(value: StoredValue<JsonValue>) -> JsonValue {
    match value {
        StoredValue::Text(text) => JsonValue::String(text.expose_secret().to_owned()),
        StoredValue::Other(other) => other,
    }
}
