use std::error::Error as StdError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockInteractiveError<Transport>
where
    Transport: StdError + 'static,
{
    #[error(transparent)]
    Responses(#[from] super::responses::Error),
    #[error(transparent)]
    Passthrough(#[from] super::passthrough::Error),
    #[error(transparent)]
    Search(#[from] super::search::Error),
    #[error(transparent)]
    VectorRetrieve(#[from] super::vector_retrieve::Error),
    #[error(transparent)]
    RealtimeBridge(#[from] super::realtime_bridge::Error<Transport>),
    #[error(
        "betterlite does not run the {operation} operation through the Bedrock interactive provider"
    )]
    Unsupported { operation: &'static str },
}
