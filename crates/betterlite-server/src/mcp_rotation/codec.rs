use std::mem::take;

use betterlite_db::mcp_server_encoding::credentials::{CredentialBlob, credentials_blob_to_dict};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::blob::decode_legacy_base64_text;
use super::error::RotationCodecError;

// The pinned credential field list; aws_region_name and aws_service_name are not secrets.
const SECRET_FIELDS: [&str; 7] = [
    "auth_value",
    "client_id",
    "client_secret",
    "client_private_key",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
];
const USER_CREDENTIAL_KEY: &str = "mcp_user_credential";
const USER_ENV_VARS_KEY: &str = "mcp_user_env_vars";
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};
const DROP_UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

#[must_use]
pub fn is_falsey(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::Bool(flag) => !flag,
        JsonValue::Number(number) => number.as_f64() == Some(0.0),
        JsonValue::String(text) => text.is_empty(),
        JsonValue::Array(items) => items.is_empty(),
        JsonValue::Object(fields) => fields.is_empty(),
    }
}

pub fn encrypt_stored_value<N, D>(
    value: JsonValue,
    cipher: &StoredCipher<N, D>,
    key: Option<&SecretString>,
) -> Result<JsonValue, SecretError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let stored = match value {
        JsonValue::String(text) => StoredValue::Text(SecretString::from(text)),
        other => StoredValue::Other(other),
    };
    Ok(match cipher.encrypt_value(stored, key)? {
        StoredValue::Text(sealed) => JsonValue::String(sealed.expose_secret().to_owned()),
        StoredValue::Other(other) => other,
    })
}

pub fn encrypt_credentials<N, D>(
    credentials: &mut CredentialBlob,
    cipher: &StoredCipher<N, D>,
    key: Option<&SecretString>,
) -> Result<(), SecretError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    for field in SECRET_FIELDS {
        let Some(value) = credentials.get_mut(field) else {
            continue;
        };
        *value = encrypt_stored_value(take(value), cipher, key)?;
    }
    Ok(())
}

pub fn decrypt_credentials<N, D>(credentials: &mut CredentialBlob, cipher: &StoredCipher<N, D>)
where
    D: SecretDiagnostics,
{
    for field in SECRET_FIELDS {
        let Some(JsonValue::String(text)) = credentials.get_mut(field) else {
            continue;
        };
        let stored = StoredValue::<()>::Text(SecretString::from(take(text)));
        if let Some(StoredValue::Text(plaintext)) =
            cipher.decrypt_value(stored, field, KEEP_ORIGINAL)
        {
            plaintext.expose_secret().clone_into(text);
        }
    }
}

pub fn reencrypt_credentials_blob<N, D>(
    credentials: Option<&JsonValue>,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<Option<JsonValue>, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(credentials) = credentials else {
        return Ok(None);
    };
    if is_falsey(credentials) {
        return Ok(None);
    }
    let mut blob: CredentialBlob = credentials_blob_to_dict(credentials)?;
    decrypt_credentials(&mut blob, cipher);
    encrypt_credentials(&mut blob, cipher, Some(new_key))?;
    let text = serde_json::to_string(&JsonValue::Object(blob)).map_err(RotationCodecError::Json)?;
    Ok(Some(JsonValue::String(text)))
}

pub fn decode_user_credential<N, D>(stored: &str, cipher: &StoredCipher<N, D>) -> Option<String>
where
    D: SecretDiagnostics,
{
    let value = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    match cipher.decrypt_value(value, USER_CREDENTIAL_KEY, DROP_UNREADABLE) {
        Some(StoredValue::Text(plaintext)) => Some(plaintext.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => decode_legacy_base64_text(stored).ok(),
    }
}

pub fn reencrypt_user_credential<N, D>(
    stored: &str,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<Option<JsonValue>, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(plaintext) = decode_user_credential(stored, cipher) else {
        return Ok(None);
    };
    Ok(Some(encrypt_stored_value(
        JsonValue::String(plaintext),
        cipher,
        Some(new_key),
    )?))
}

pub fn reencrypt_user_env_var<N, D>(
    stored: &str,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<Option<JsonValue>, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let value = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    match cipher.decrypt_value(value, USER_ENV_VARS_KEY, DROP_UNREADABLE) {
        Some(StoredValue::Text(plaintext)) => Ok(Some(encrypt_stored_value(
            JsonValue::String(plaintext.expose_secret().to_owned()),
            cipher,
            Some(new_key),
        )?)),
        Some(StoredValue::Other(())) | None => Ok(None),
    }
}
