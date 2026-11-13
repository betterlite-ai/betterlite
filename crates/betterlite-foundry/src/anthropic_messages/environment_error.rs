use std::error::Error as StdError;

use betterlite_azure::credentials::selection::AzureTokenError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvironmentError<B: StdError + 'static> {
    #[error(transparent)]
    Token(#[from] AzureTokenError),
    #[error(transparent)]
    Betas(B),
}
