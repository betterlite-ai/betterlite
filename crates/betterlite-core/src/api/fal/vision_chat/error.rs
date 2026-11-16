use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use thiserror::Error;

use crate::api::fal::images::error::FalImageError;

#[derive(Debug, Error)]
pub enum FalAIError {
    #[error(
        "fal_ai chat completions accept exactly one user message; system prompts and multi-turn history are not supported"
    )]
    OneUserMessage,
    #[error("fal_ai chat completions require text in the user message")]
    TextRequired,
    #[error(
        "fal_ai chat completions require exactly one image_url content part in the user message"
    )]
    OneImageRequired,
    #[error("fal_ai chat completions do not support streaming")]
    StreamingUnsupported,
    #[error("the stream option must be a boolean")]
    StreamNotBoolean,
    #[error("Unsupported reasoning_effort {value:?} for {model}")]
    UnsupportedReasoningEffort { value: Json, model: String },
    // LiteLLM raises a plain ValueError here, so the error carries no HTTP status
    #[error("FAL_AI_API_KEY is not set")]
    MissingApiKey(#[source] FalImageError),
    #[error("fal_ai returned an unexpected response body: {text}")]
    UnexpectedResponse {
        text: String,
        headers: ImageHeaders,
        // None when the body parsed and only the token total overflowed
        #[source]
        source: Option<serde_json::Error>,
    },
    // what get_error_class builds for an upstream error: the status, message and headers as the provider sent them
    #[error("{message}")]
    Upstream {
        status: u16,
        message: String,
        headers: ImageHeaders,
    },
    #[error("the completion record has no choice to fill")]
    NoChoices,
}

impl FalAIError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Upstream { status, .. } => Some(*status),
            Self::MissingApiKey(_) | Self::NoChoices => None,
            Self::UnexpectedResponse { .. } => Some(422),
            Self::OneUserMessage
            | Self::TextRequired
            | Self::OneImageRequired
            | Self::StreamingUnsupported
            | Self::StreamNotBoolean
            | Self::UnsupportedReasoningEffort { .. } => Some(400),
        }
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&ImageHeaders> {
        if let Self::UnexpectedResponse { headers, .. } | Self::Upstream { headers, .. } = self {
            return Some(headers);
        }
        None
    }
}
