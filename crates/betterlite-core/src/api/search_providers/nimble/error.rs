use betterlite_openai::error::ProviderError;
use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NimbleSearchError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("NIMBLE_API_KEY is not set. Set `NIMBLE_API_KEY` environment variable.")]
    MissingApiKey,
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}
