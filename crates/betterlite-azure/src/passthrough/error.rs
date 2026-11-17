use betterlite_openai::error::ProviderError;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use thiserror::Error;

use crate::azure_urls::AzureUrlError;
use crate::credentials::selection::AzureTokenError;

#[derive(Debug, Error)]
pub enum AzurePassthroughError {
    #[error("Azure api base not found")]
    MissingApiBase,
    #[error(transparent)]
    Url(#[from] AzureUrlError),
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error(transparent)]
    Token(#[from] AzureTokenError),
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl From<ProviderError> for AzurePassthroughError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}
