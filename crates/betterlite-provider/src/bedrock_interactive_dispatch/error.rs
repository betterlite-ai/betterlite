use std::error::Error as StdError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockInteractiveError<Transport>
where
    Transport: StdError + 'static,
{
    #[error(transparent)]
    Responses(#[from] super::responses::OperationError),
    #[error(transparent)]
    Passthrough(#[from] super::passthrough::OperationError),
    #[error(transparent)]
    Search(#[from] super::search::OperationError),
    #[error(transparent)]
    VectorRetrieve(#[from] super::vector_retrieve::OperationError),
    #[error(transparent)]
    RealtimeBridge(#[from] super::realtime_bridge::OperationError<Transport>),
    #[error(
        "betterlite does not run the {operation} operation through the Bedrock interactive provider"
    )]
    Unsupported { operation: &'static str },
}
