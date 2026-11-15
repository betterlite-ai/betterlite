use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;

use super::reply::ParallelAiReplyError;

#[derive(Debug, Error)]
pub enum ParallelAiSearchError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("PARALLEL_API_KEY is not set. Set `PARALLEL_API_KEY` environment variable.")]
    MissingApiKey,
    #[error("the `{field}` search option has the wrong type")]
    OptionType { field: &'static str },
    #[error(transparent)]
    Reply(#[from] ParallelAiReplyError),
}
