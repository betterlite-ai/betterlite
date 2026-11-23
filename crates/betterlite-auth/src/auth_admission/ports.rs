use std::error::Error;

use async_trait::async_trait;
use indexmap::IndexMap;
use thiserror::Error;

use crate::managed_admission::error::AdmissionError;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// B4781's calls leave the delegation and billing marks on the auth, so the caller hands it over mutably
#[async_trait]
pub trait ManagedAdmission: Send + Sync {
    async fn admit(&self, auth: &mut UserAPIKeyAuth) -> Result<(), AdmissionError>;

    async fn prepare(
        &self,
        auth: &mut UserAPIKeyAuth,
        target_name: &str,
        billable: bool,
    ) -> Result<(), AdmissionError>;
}

// LiteLLM resolves, filters and sets the destinations in one synchronous pass while auth stays open
pub trait TenantDestinations: Send + Sync {
    fn seed(
        &self,
        auth: &UserAPIKeyAuth,
        headers: &IndexMap<String, String>,
    ) -> Result<(), DestinationSeedError>;
}

// B855 logs this text at debug, so it is the cause alone and never a header or callback variable
#[derive(Debug, Error)]
#[error("{source}")]
pub struct DestinationSeedError {
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}
