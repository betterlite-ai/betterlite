use crate::chat_choices::stream::StreamError;
use crate::chat_content::ChatContentError;
use crate::compatible_wire::error::CompatibleWireError;
use crate::error::OpenAiError;
use thiserror::Error;

use super::capabilities::CapabilityError;

#[derive(Debug, Error)]
pub enum GroqChatError {
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error(transparent)]
    Reply(#[from] CompatibleWireError),
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error(transparent)]
    Provider(#[from] OpenAiError),
    #[error("{message}")]
    BadRequest { message: String },
    #[error("response_format.json_schema must be an object with a schema")]
    SchemaShape,
}

impl GroqChatError {
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::BadRequest { .. } | Self::SchemaShape => 400,
            Self::Content(error) => error.status_code(),
            Self::Provider(OpenAiError::Provider(error)) => error.status_code,
            Self::Stream(StreamError::Provider { status, .. }) => *status,
            Self::Capability(_) | Self::Reply(_) | Self::Provider(_) | Self::Stream(_) => 500,
        }
    }
}
