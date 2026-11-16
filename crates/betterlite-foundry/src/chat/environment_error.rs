use betterlite_azure::credentials::selection::AzureTokenError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error(transparent)]
    Token(#[from] AzureTokenError),
}
