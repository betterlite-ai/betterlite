use std::error::Error;

use betterlite_models::resolution::ResolutionError;

use crate::deployment_credentials::error::CredentialError;
use crate::model_catalog_lookup::error::LookupError;

// What a router metadata read raises: LiteLLM's two ValueErrors, the lookups it propagates and the metadata source's failure.
#[derive(Debug, thiserror::Error)]
pub enum RouterMetadataError<SourceError>
where
    SourceError: Error + 'static,
{
    #[error("Deployment not found")]
    DeploymentNotFound,
    #[error(
        "Deployment missing valid litellm_params. Got: {params_type}, deployment_id: {deployment_id}"
    )]
    InvalidParameters {
        params_type: &'static str,
        deployment_id: String,
    },
    #[error("{field} is not a string")]
    FieldNotText { field: &'static str },
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("{source}")]
    Source {
        #[source]
        source: SourceError,
    },
}

// Why one deployment's refresh stopped before it wrote; the worker reports every variant with the same debug line.
#[derive(Debug)]
pub enum RefreshFailure {
    Credentials(CredentialError),
    Params,
    Headers,
    Resolution(ResolutionError),
}
