use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::McpEncodingError;
use crate::mcp_stored_contracts::{JsonValue, MCPAuth};
use crate::stored_cipher::DbCipher;

pub type CredentialBlob = BTreeMap<String, JsonValue>;

const SECRET_FIELDS: [&str; 7] = [
    "auth_value",
    "client_id",
    "client_secret",
    "client_private_key",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
];
const MINTED_TOKEN_FIELDS: [&str; 3] = ["access_token", "refresh_token", "expires_in"];
const DECRYPT_POLICY: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

pub(crate) const TOKEN_EXCHANGE_COLUMN_FIELDS: [&str; 4] = [
    "token_exchange_endpoint",
    "audience",
    "subject_token_type",
    "token_exchange_profile",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialClass {
    ClientForwarded,
    Auth(MCPAuth),
}

#[must_use]
pub const fn credential_auth_class(auth_type: Option<MCPAuth>) -> Option<CredentialClass> {
    match auth_type {
        Some(MCPAuth::TruePassthrough | MCPAuth::OauthDelegate) => {
            Some(CredentialClass::ClientForwarded)
        }
        Some(class) => Some(CredentialClass::Auth(class)),
        None => None,
    }
}

pub fn encrypt_credentials<N, D>(
    credentials: &mut CredentialBlob,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    for field in SECRET_FIELDS {
        let Some(value) = credentials.get_mut(field) else {
            continue;
        };
        let stored = match value {
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

pub fn decrypt_credentials(credentials: &mut CredentialBlob, cipher: &DbCipher) {
    for field in SECRET_FIELDS {
        let Some(JsonValue::String(text)) = credentials.get_mut(field) else {
            continue;
        };
        let stored = StoredValue::<()>::Text(SecretString::from(std::mem::take(text)));
        if let Some(StoredValue::Text(plaintext)) =
            cipher.decrypt_value(stored, field, DECRYPT_POLICY)
        {
            *text = plaintext.expose_secret().to_owned();
        }
    }
}

pub fn drop_stale_minted_tokens(merged: &mut CredentialBlob, new_credentials: &CredentialBlob) {
    if !new_credentials.contains_key("client_id") && !new_credentials.contains_key("client_secret")
    {
        return;
    }
    merged.retain(|field, _value| {
        !MINTED_TOKEN_FIELDS.contains(&field.as_str()) || new_credentials.contains_key(field)
    });
}

pub fn credentials_blob_to_dict(blob: &JsonValue) -> Result<CredentialBlob, McpEncodingError> {
    match blob {
        JsonValue::Object(fields) => Ok(fields.clone()),
        JsonValue::String(text) => {
            let parsed: JsonValue = serde_json::from_str(text).map_err(McpEncodingError::Json)?;
            match parsed {
                JsonValue::Object(fields) => Ok(fields),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Array(_) => Err(McpEncodingError::CredentialsNotObject),
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(McpEncodingError::CredentialsNotObject)
        }
    }
}
