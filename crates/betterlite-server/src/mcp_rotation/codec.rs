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
pub const USER_CREDENTIAL_KEY: &str = "mcp_user_credential";
pub const USER_ENV_VARS_KEY: &str = "mcp_user_env_vars";
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

fn stored_value(value: JsonValue) -> StoredValue<JsonValue> {
    match value {
        JsonValue::String(text) => StoredValue::Text(SecretString::from(text)),
        other => StoredValue::Other(other),
    }
}

pub fn sealed_text(sealed: StoredValue<JsonValue>) -> JsonValue {
    match sealed {
        StoredValue::Text(sealed) => JsonValue::String(sealed.expose_secret().to_owned()),
        StoredValue::Other(other) => other,
    }
}

fn legacy_text(stored: &str) -> Option<SecretString> {
    decode_legacy_base64_text(stored)
        .ok()
        .map(SecretString::from)
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
        *value = sealed_text(cipher.encrypt_value(stored_value(take(value)), key)?);
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
        let plaintext = cipher.decrypt_value(stored, field, KEEP_ORIGINAL);
        if let Some(StoredValue::Text(plaintext)) = plaintext {
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

pub fn decode_user_value<N, D>(
    text: &str,
    key: &str,
    cipher: &StoredCipher<N, D>,
    legacy_base64: bool,
) -> Option<SecretString>
where
    D: SecretDiagnostics,
{
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match cipher.decrypt_value(stored, key, DROP_UNREADABLE) {
        Some(StoredValue::Text(plaintext)) => Some(plaintext),
        Some(StoredValue::Other(())) | None if legacy_base64 => legacy_text(text),
        Some(StoredValue::Other(())) | None => None,
    }
}

pub fn reencrypt_user_value<N, D>(
    plaintext: Option<SecretString>,
    cipher: &StoredCipher<N, D>,
    new_key: &SecretString,
) -> Result<Option<JsonValue>, RotationCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    plaintext
        .map(|text| cipher.encrypt_value(StoredValue::Text(text), Some(new_key)))
        .transpose()
        .map(|sealed| sealed.map(sealed_text))
        .map_err(RotationCodecError::Secret)
}
