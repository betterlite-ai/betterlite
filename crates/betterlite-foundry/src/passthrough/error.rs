use betterlite_http::user_url::error::InvalidUrl;
use betterlite_openai::error::ProviderError;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use thiserror::Error;

use crate::credentials::error::CredentialsError;

#[derive(Debug, Error)]
pub enum FoundryPassthroughError {
    #[error("Azure AI api base not found: set `api_base` on the deployment or AZURE_AI_API_BASE")]
    MissingApiBase,
    #[error("the Azure AI api base is not a valid URL")]
    InvalidApiBase(#[source] InvalidUrl),
    #[error("the Azure AI api base path cannot be rewritten as a Foundry root")]
    UnwritableBasePath,
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl From<ProviderError> for FoundryPassthroughError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}
