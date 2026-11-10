use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CountTokensError {
    #[error("model parameter is required")]
    ModelRequired,
    #[error("messages parameter is required for Converse input")]
    MessagesRequired,
    #[error("Message {index} must be a dictionary")]
    MessageNotObject { index: usize },
    #[error("Message {index} must have a '{field}' field")]
    MessageFieldMissing { index: usize, field: &'static str },
    #[error("Request must contain content to count tokens")]
    ContentRequired,
}
