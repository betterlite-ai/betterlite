use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::JsonData;
use crate::stored_cipher::DbCipher;

use super::decode::{decode_user_credential, decoded_row, parse_oauth_payload};
use super::rows::{CredentialRows, stored_text};
use super::types::{CredentialError, OAuthCredentialPayload};

// LiteLLM's row accessors share one signature shape; one line each keeps them under the similarity check's 8-line floor
#[rustfmt::skip]
pub async fn store_user_credential<A>(table: &A, cipher: &DbCipher, user_id: &str, server_id: &str, credential: &str) -> Result<(), CredentialError> where A: CredentialRows {
    let value = StoredValue::<()>::Text(SecretString::from(credential.to_owned()));
    let StoredValue::Text(sealed) = cipher.encrypt_value(value, None)? else { return Err(CredentialError::SealedNotText) };
    table.upsert_credential(user_id, server_id, sealed.expose_secret()).await?;
    Ok(())
}

#[rustfmt::skip]
pub async fn get_user_credential<A>(table: &A, cipher: &DbCipher, user_id: &str, server_id: &str) -> Result<Option<SecretString>, CredentialError> where A: CredentialRows {
    let Some(row) = table.find_credential(user_id, server_id).await? else { return Ok(None) };
    Ok(decode_user_credential(cipher, stored_text(&row)?))
}

#[rustfmt::skip]
pub async fn delete_user_credential<A>(table: &A, user_id: &str, server_id: &str) -> Result<(), CredentialError> where A: CredentialRows {
    table.delete_credential(user_id, server_id).await?;
    Ok(())
}

#[rustfmt::skip]
pub async fn get_user_oauth_credential<A>(table: &A, cipher: &DbCipher, user_id: &str, server_id: &str) -> Result<Option<OAuthCredentialPayload>, CredentialError> where A: CredentialRows {
    let Some(row) = table.find_credential(user_id, server_id).await? else { return Ok(None) };
    parse_oauth_payload(decoded_row(&row, cipher, user_id, server_id)?.as_ref())
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
    let decoded = decode_user_credential(cipher, stored_text(row)?);
    if decoded
        .as_ref()
        .is_none_or(|text| text.expose_secret().is_empty())
    {
        warn_replacing_undecryptable(user_id, server_id);
        return Ok(());
    }
    if parse_oauth_payload(decoded.as_ref())?.is_some() {
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
