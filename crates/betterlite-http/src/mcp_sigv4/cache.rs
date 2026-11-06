use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aws_credential_types::Credentials;
use aws_credential_types::provider::{ProvideCredentials as _, SharedCredentialsProvider};
use aws_smithy_async::time::SharedTimeSource;

use super::error::McpSigV4Error;

// botocore refreshes temporary credentials once they are this close to expiring
const REFRESH_BEFORE_EXPIRY: Duration = Duration::from_mins(10);

pub struct CredentialCache {
    pub clock: SharedTimeSource,
    pub provider: SharedCredentialsProvider,
    pub current: Mutex<Option<Credentials>>,
}

impl CredentialCache {
    pub async fn get(&self) -> Result<Credentials, McpSigV4Error> {
        let now = self.clock.now();
        let cached = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(valid) = cached
            && valid.expiry().is_none_or(|expiry| {
                expiry
                    .duration_since(now)
                    .is_ok_and(|remaining| remaining > REFRESH_BEFORE_EXPIRY)
            })
        {
            return Ok(valid);
        }
        let fresh = self
            .provider
            .provide_credentials()
            .await
            .map_err(|source| McpSigV4Error::Provider { source })?;
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh.clone());
        Ok(fresh)
    }
}
