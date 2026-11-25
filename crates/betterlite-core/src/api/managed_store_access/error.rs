use std::convert::Infallible;
use std::error::Error;

use betterlite_auth::auth_resource_grants::managed_rows::ManagedVectorStoreLookupError;
use thiserror::Error;

use crate::api::vector_store_access::error::VectorStoreAccessError;

// the detail both fail-closed resolution arms answer with
pub const UNABLE_TO_VALIDATE_DETAIL: &str = "Unable to validate vector store access";

// the detail assert_user_can_access_vector_store answers with when the caller supplies none
pub const ACCESS_DENIED_DETAIL: &str =
    "Access denied: You do not have permission to access this vector store";

// the policy answer B1060's can_user_access_vector_store gives a route: a denial or its own failure
#[derive(Debug, Error)]
pub enum ManagedStorePolicyError {
    #[error("{detail}")]
    Denied { detail: String },
    #[error(transparent)]
    Access(#[from] VectorStoreAccessError),
}

impl ManagedStorePolicyError {
    // LiteLLM raises 403 for the denial and passes the policy failure's own status through
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::Denied { .. } => 403,
            Self::Access(error) => error.http().unwrap_or(500),
        }
    }

    // HTTPException's detail: the denial's text, or the policy failure's message
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Denied { detail } => detail.clone(),
            Self::Access(error) => error.to_string(),
        }
    }
}

// what failed inside one resolution arm: the registry's own error, or the shared lookup's
#[derive(Debug, Error)]
pub enum ResolutionSource<RegistryError = Infallible>
where
    RegistryError: Error + 'static,
{
    #[error(transparent)]
    Registry(RegistryError),
    #[error(transparent)]
    Lookup(#[from] ManagedVectorStoreLookupError),
    #[error("the managed vector store row is not a readable record")]
    Record(#[source] serde_json::Error),
    #[error(transparent)]
    Params(VectorStoreAccessError),
}

// the failures of the managed store lookup and the ownership assertions
#[derive(Debug, Error)]
pub enum ManagedStoreAccessError<RegistryError = Infallible>
where
    RegistryError: Error + 'static,
{
    #[error("Failed to resolve vector store id={vector_store_id} from registry: {source}")]
    RegistryFailure {
        vector_store_id: String,
        #[source]
        source: ResolutionSource<RegistryError>,
    },
    #[error("Failed to resolve vector store id={vector_store_id} from shared cache: {source}")]
    SharedCacheFailure {
        vector_store_id: String,
        #[source]
        source: ResolutionSource<RegistryError>,
    },
    #[error(transparent)]
    Policy(#[from] ManagedStorePolicyError),
}

impl<RegistryError> ManagedStoreAccessError<RegistryError>
where
    RegistryError: Error + 'static,
{
    // the status a route answers with: both resolution arms fail closed with a 500
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::RegistryFailure { .. } | Self::SharedCacheFailure { .. } => 500,
            Self::Policy(error) => error.status(),
        }
    }

    // HTTPException's detail text
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::RegistryFailure { .. } | Self::SharedCacheFailure { .. } => {
                String::from(UNABLE_TO_VALIDATE_DETAIL)
            }
            Self::Policy(error) => error.detail(),
        }
    }
}
