use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;

use crate::rds_credentials::RdsEndpoint;
use crate::rds_credentials::error::RdsCredentialsError;

pub const AZURE_POSTGRESQL_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";

pub trait AzureTokenProvider: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn token(&self) -> Result<SecretString, Self::Error>;
}

// the factory receives the PostgreSQL scope explicitly, so an AZURE_SCOPE intended for another service never changes it
pub trait AzureProviderFactory: Send + Sync {
    type Provider: AzureTokenProvider<Error = Self::Error>;
    type Error: Error + Send + Sync + 'static;

    fn build(&self, scope: &str) -> Result<Arc<Self::Provider>, Self::Error>;
}

pub trait RdsTokenProvider: Send + Sync {
    fn mint(&self, endpoint: &RdsEndpoint<'_>) -> Result<SecretString, RdsCredentialsError>;
}

pub struct AzureProviderCache<Factory>
where
    Factory: AzureProviderFactory,
{
    factory: Arc<Factory>,
    cached: Mutex<Option<Arc<Factory::Provider>>>,
}

impl<Factory> AzureProviderCache<Factory>
where
    Factory: AzureProviderFactory,
{
    #[must_use]
    pub const fn new(factory: Arc<Factory>) -> Self {
        Self {
            factory,
            cached: Mutex::new(None),
        }
    }

    // a failed construction is never stored, so the next writer, reader or refresh call tries again
    pub fn provider(&self) -> Result<Arc<Factory::Provider>, Factory::Error> {
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
