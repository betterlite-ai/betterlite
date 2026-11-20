use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockInteractiveError {
    #[error(transparent)]
    Responses(#[from] super::responses::Error),
    #[error(transparent)]
    Passthrough(#[from] super::passthrough::Error),
    #[error(transparent)]
    Search(#[from] super::search::Error),
    #[error(transparent)]
    VectorRetrieve(#[from] super::vector_retrieve::Error),
    #[error(transparent)]
    RealtimeBridge(#[from] super::realtime_bridge::Error),
    #[error(
        "betterlite does not run the {operation} operation through the Bedrock interactive provider"
    )]
    Unsupported { operation: &'static str },
}
