use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;

use crate::rds_credentials::RdsEndpoint;
use crate::rds_credentials::error::RdsCredentialsError;

pub const AZURE_POSTGRESQL_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";

// Azure's error type lives in betterlite-azure, which already reaches this crate through betterlite-config, so the cause crosses the seam as its message
#[derive(Debug, thiserror::Error)]
pub enum AzureProviderError {
    #[error("{message}")]
    Provider { message: String },
}

pub trait AzureTokenProvider: Send + Sync {
    fn token(&self) -> Result<SecretString, AzureProviderError>;
}

// the factory receives the PostgreSQL scope explicitly, so an AZURE_SCOPE intended for another service never changes it
pub trait AzureProviderFactory: Send + Sync {
    fn build(&self, scope: &str) -> Result<Arc<dyn AzureTokenProvider>, AzureProviderError>;
}

pub trait RdsTokenProvider: Send + Sync {
    fn mint(&self, endpoint: &RdsEndpoint<'_>) -> Result<SecretString, RdsCredentialsError>;
}

pub struct AzureProviderCache {
    factory: Arc<dyn AzureProviderFactory>,
    cached: Mutex<Option<Arc<dyn AzureTokenProvider>>>,
}

impl AzureProviderCache {
    #[must_use]
    pub fn new(factory: Arc<dyn AzureProviderFactory>) -> Self {
        Self {
            factory,
            cached: Mutex::new(None),
        }
    }

    // a failed construction is never stored, so the next writer, reader or refresh call tries again
    pub fn provider(&self) -> Result<Arc<dyn AzureTokenProvider>, AzureProviderError> {
        {
            let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(provider) = cached.as_ref() {
                return Ok(Arc::clone(provider));
            }
        }
        let built = self.factory.build(AZURE_POSTGRESQL_SCOPE)?;
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        *cached = Some(Arc::clone(&built));
        drop(cached);
        Ok(built)
    }
}
