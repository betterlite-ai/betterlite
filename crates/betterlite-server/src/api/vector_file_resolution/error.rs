use betterlite_core::api::file_upload_options::error::FileUploadOptionsError;
use betterlite_core::api::model_credentials::CredentialLookupError;
use std::convert::Infallible;
use std::error::Error;
use thiserror::Error;

// The ways resolution fails: B1021's managed-id answer and B10060's credential answer keep their own status and detail.
#[derive(Debug, Error)]
pub enum VectorFileResolutionError<CatalogError = Infallible, GrantError = Infallible>
where
    CatalogError: Error + 'static,
    GrantError: Error + 'static,
{
    #[error(transparent)]
    ManagedId(#[from] FileUploadOptionsError),
    // B1020's 403 keeps its status, type, param and code
    #[error(transparent)]
    Authorization(GrantError),
    #[error(transparent)]
    Lookup(#[from] CredentialLookupError<CatalogError>),
    #[error("the deployment credentials for `{model_id}` could not be read")]
    Catalog {
        model_id: String,
        #[source]
        source: CatalogError,
    },
    #[error("the caller's key document could not be read")]
    Caller(#[source] serde_json::Error),
    #[error("the vector store record could not be read")]
    Store(#[source] serde_json::Error),
    #[error("`litellm_params` in the vector store record is not an object")]
    StoreParams,
}
