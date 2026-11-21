mod tests;

use std::convert::Infallible;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_db::sso_assertion_rows::AssertionRows;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::sso_assertions::assertion::{AssertionError, SsoIdentityAssertion, encode_payload};
use crate::sso_assertions::cache::SsoAssertionCache;

// LiteLLM's retain hook writes this line and swallows the failure, because the login itself must not fail on it.
const RETAIN_FAILURE_WARNING: &str = "Failed to persist the SSO identity assertion for EMA egress";

// No #[from]: From<Store> would overlap From<SecretError> at Store = SecretError (E0119).
#[derive(Debug, Error)]
pub enum SsoAssertionStoreError<Store>
where
    Store: StdError + Send + Sync + 'static,
{
    #[error("the SSO identity assertion payload could not be encoded: {0}")]
    Encode(#[source] AssertionError),
    #[error("the SSO identity assertion could not be encrypted: {0}")]
    Cipher(#[source] SecretError),
    #[error("the SSO identity assertion rows failed: {0}")]
    Store(#[source] Store),
}

pub struct DbSsoAssertionStore<R: AssertionRows> {
    pub rows: Option<R>,
    pub cipher: DbCipher,
    pub cache: Arc<SsoAssertionCache>,
    pub config_declares_id_jag: bool,
}

impl<R> DbSsoAssertionStore<R>
where
    R: AssertionRows + Send + Sync,
{
    pub async fn persist(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), SsoAssertionStoreError<R::Error>> {
        let Some(rows) = self.rows.as_ref() else {
            return Ok(());
        };
        let payload = encode_payload(assertion).map_err(SsoAssertionStoreError::Encode)?;
        let sealed = self
            .cipher
            .encrypt_value(
                StoredValue::<Infallible>::Text(SecretString::from(payload)),
                None,
            )
            .map_err(SsoAssertionStoreError::Cipher)?;
        let ciphertext = match sealed {
            StoredValue::Text(text) => text,
            StoredValue::Other(never) => match never {},
        };
        rows.upsert(user_id, ciphertext.expose_secret())
            .await
            .map_err(SsoAssertionStoreError::Store)?;
        // only after the write: an invalidate first would let a reader cache the row this write replaces
        self.cache.invalidate(user_id);
        Ok(())
    }

    pub async fn retention_enabled(&self) -> Result<bool, SsoAssertionStoreError<R::Error>> {
        // the config declaration answers without a query, the same short circuit LiteLLM takes
        if self.config_declares_id_jag {
            return Ok(true);
        }
        let Some(rows) = self.rows.as_ref() else {
            return Ok(false);
        };
        rows.id_jag_server_exists()
            .await
            .map_err(SsoAssertionStoreError::Store)
    }

    pub async fn retain(&self, user_id: &str, assertion: Option<&SsoIdentityAssertion>) {
        let Some(captured) = assertion else {
            return;
        };
        let enabled = match self.retention_enabled().await {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!("{RETAIN_FAILURE_WARNING} (user_id={user_id}): {error}");
                return;
            }
        };
        if !enabled {
            return;
        }
        if let Err(error) = self.persist(user_id, captured).await {
            tracing::warn!("{RETAIN_FAILURE_WARNING} (user_id={user_id}): {error}");
        }
    }
}
