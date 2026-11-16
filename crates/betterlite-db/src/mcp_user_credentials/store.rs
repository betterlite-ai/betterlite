use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::JsonData;
use crate::stored_cipher::DbCipher;

use super::decode::{decode_user_credential, parse_oauth_payload, warn_undecryptable_credential};
use super::rows::{CredentialRows, text_field};
use super::types::{CredentialError, OAuthCredentialPayload};

const CREDENTIAL_COLUMN: &str = "credential_b64";

pub async fn store_user_credential<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
    credential: &str,
) -> Result<(), CredentialError> {
    let value = StoredValue::<()>::Text(SecretString::from(credential.to_owned()));
    let StoredValue::Text(sealed) = cipher.encrypt_value(value, None)? else {
        return Err(CredentialError::SealedNotText);
    };
    table
        .upsert_credential(user_id, server_id, sealed.expose_secret())
        .await?;
    Ok(())
}

pub async fn get_user_credential<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
) -> Result<Option<String>, CredentialError> {
    let Some(row) = table.find_credential(user_id, server_id).await? else {
        return Ok(None);
    };
    let stored = text_field(&row, CREDENTIAL_COLUMN).ok_or(CredentialError::MissingField {
        field: CREDENTIAL_COLUMN,
    })?;
    Ok(decode_user_credential(cipher, stored))
}

pub async fn delete_user_credential<A: CredentialRows>(
    table: &A,
    user_id: &str,
    server_id: &str,
) -> Result<(), CredentialError> {
    table.delete_credential(user_id, server_id).await?;
    Ok(())
}

pub async fn get_user_oauth_credential<A: CredentialRows>(
    table: &A,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
) -> Result<Option<OAuthCredentialPayload>, CredentialError> {
    let Some(row) = table.find_credential(user_id, server_id).await? else {
        return Ok(None);
    };
    let stored = text_field(&row, CREDENTIAL_COLUMN).ok_or(CredentialError::MissingField {
        field: CREDENTIAL_COLUMN,
    })?;
    let decoded = decode_user_credential(cipher, stored);
    if decoded.is_none() {
        warn_undecryptable_credential(user_id, server_id);
    }
    parse_oauth_payload(decoded.as_deref())
}

// LiteLLM lets a new OAuth2 grant replace a row that no longer decrypts, an empty one, and another grant, and refuses a readable BYOK key so an authorization cannot destroy an API key
pub(super) fn guard_byok_overwrite(
    cipher: &DbCipher,
    existing: Option<&JsonData>,
    user_id: &str,
    server_id: &str,
) -> Result<(), CredentialError> {
    let Some(row) = existing else {
        return Ok(());
    };
    let stored = text_field(row, CREDENTIAL_COLUMN).ok_or(CredentialError::MissingField {
        field: CREDENTIAL_COLUMN,
    })?;
    let decoded = decode_user_credential(cipher, stored);
    if decoded.as_deref().is_none_or(str::is_empty) {
        warn_replacing_undecryptable(user_id, server_id);
        return Ok(());
    }
    if parse_oauth_payload(decoded.as_deref())?.is_some() {
        return Ok(());
    }
    Err(CredentialError::RefusesByokOverwrite {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    })
}

fn warn_replacing_undecryptable(user_id: &str, server_id: &str) {
    tracing::warn!(
        user_id,
        server_id,
        "store_user_oauth_credential: the existing credential could not be decrypted (likely written under a previous LITELLM_SALT_KEY); replacing it with the newly authorized OAuth2 token"
    );
}
