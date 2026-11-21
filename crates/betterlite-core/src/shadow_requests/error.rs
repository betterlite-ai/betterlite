use betterlite_redact::error::RedactionError;
use thiserror::Error;

use crate::api::messages_chat_transform::error::TransformError;
use crate::api::responses_chat_options::error::ChatOptionsError;

#[derive(Debug, Error)]
pub enum ShadowRequestError {
    #[error("messages is not an array of message objects")]
    MessagesNotObjects,
    #[error("model is not text")]
    ModelNotText,
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error(transparent)]
    ChatOptions(#[from] ChatOptionsError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
}
