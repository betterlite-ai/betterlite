use std::convert::Infallible;
use std::error::Error as StdError;

use betterlite_db::sso_assertion_rows::{
    ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY, AssertionRow, AssertionRows,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

// The store's failure type belongs to the caller, so it is carried rather than boxed.
#[derive(Debug, Error)]
pub enum RotationError<Store>
where
    Store: StdError + Send + Sync + 'static,
{
    #[error("the SSO identity assertion could not be re-encrypted: {0}")]
    Cipher(#[from] SecretError),
    #[error("the SSO identity assertion rows failed: {0}")]
    Store(#[from] Store),
}

// LiteLLM's rotate_sso_identity_assertions_master_key: one row at a time, so no whole-table plaintext.
pub async fn rotate_assertions<R>(
    rows: &R,
    cipher: &DbCipher,
    new_master_key: &SecretString,
) -> Result<(), RotationError<R::Error>>
where
    R: AssertionRows + Sync,
{
    let mut rotated = 0_usize;
    let mut skipped = 0_usize;
    for AssertionRow {
        user_id,
        assertion_b64,
    } in rows.find_all().await?
    {
        let stored = StoredValue::<Infallible>::Text(SecretString::from(assertion_b64));
        let Some(StoredValue::Text(plaintext)) =
            cipher.decrypt_value(stored, ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY)
        else {
            tracing::warn!(
                "rotate_sso_identity_assertions_master_key: could not decrypt assertion for user_id={user_id}, skipping"
            );
            skipped = skipped.saturating_add(1);
            continue;
        };
        let sealed = match cipher.encrypt_value(
            StoredValue::<Infallible>::Text(plaintext),
            Some(new_master_key),
        )? {
            StoredValue::Text(ciphertext) => ciphertext,
            StoredValue::Other(never) => match never {},
        };
        rows.update(&user_id, sealed.expose_secret()).await?;
        rotated = rotated.saturating_add(1);
    }
    tracing::info!(
        "rotate_sso_identity_assertions_master_key: rotated {rotated} row(s), skipped {skipped}"
    );
    Ok(())
}

mod tests;
