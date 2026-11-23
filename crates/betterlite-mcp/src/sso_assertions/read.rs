mod tests;

use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;

use betterlite_db::sso_assertion_rows::{
    ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY, AssertionRows,
};
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::sso_assertions::assertion::{SsoIdentityAssertion, decode_payload};
use crate::sso_assertions::store::DbSsoAssertionStore;

#[derive(Debug, Error)]
#[error("the SSO identity assertion store is unavailable: {cause}")]
// B4664 raises this from an IdP failure too, so the cause stays a boxed error rather than a typed one
pub struct AssertionStoreUnavailable {
    #[source]
    pub cause: Box<dyn StdError + Send + Sync>,
}

pub trait SsoAssertionStore {
    fn fetch(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>> + Send;
}

impl<R> DbSsoAssertionStore<R>
where
    R: AssertionRows + Send + Sync,
{
    pub async fn fetch_uncached(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        let Some(rows) = self.rows.as_ref() else {
            return Ok(None);
        };
        let found = rows
            .find(user_id)
            .await
            .map_err(|source| AssertionStoreUnavailable {
                cause: Box::new(source),
            })?;
        let Some(row) = found else {
            return Ok(None);
        };
        let sealed = StoredValue::<Infallible>::Text(SecretString::from(row.assertion_b64));
        let opened =
            self.cipher
                .decrypt_value(sealed, ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY);
        let Some(StoredValue::Text(plaintext)) = opened else {
            return Ok(None);
        };
        match decode_payload(plaintext.expose_secret()) {
            Ok(assertion) => Ok(Some(assertion)),
            Err(_refused) => {
                tracing::warn!(
                    "Stored SSO identity assertion for user_id={user_id} could not be parsed; treating as absent."
                );
                Ok(None)
            }
        }
    }
}

impl<R> SsoAssertionStore for DbSsoAssertionStore<R>
where
    R: AssertionRows + Send + Sync,
{
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        if let Some(cached) = self.cache.get(user_id) {
            return Ok(Some(cached));
        }
        // the epoch is read before the row read, so a login that lands during it stops this cache write
        let seen_epoch = self.cache.epoch();
        let assertion = self.fetch_uncached(user_id).await?;
        if let Some(found) = assertion.as_ref() {
            self.cache.set_if_unchanged(user_id, found, seen_epoch);
        }
        Ok(assertion)
    }
}
