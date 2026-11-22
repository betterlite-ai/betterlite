use thiserror::Error;

use crate::error::{OpenAiError, ProviderError};
use crate::response_input::error::ResponseInputError;

#[derive(Debug, Error)]
pub enum PerplexityResponsesError {
    // the reply said status "failed"; the HTTP status stays as sent, 200 included
    #[error("{}", .error.message)]
    Failed { error: Box<ProviderError> },
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error(transparent)]
    Reply(#[from] OpenAiError),
}
