use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::cache_lookup::error::CacheLookupError;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum PartnerCountError {
    #[error("Unknown partner model: {model}")]
    UnknownPublisher { model: String },
    #[error("messages required for token counting")]
    MissingMessages,
    #[error("the count-tokens endpoint could not be built")]
    VertexUrl(#[source] VertexUrlError),
    #[error("the count-tokens request URL cannot be parsed")]
    Url(#[source] url::ParseError),
    #[error("a count-tokens header is not valid")]
    Header(#[source] CacheLookupError),
    #[error("the count-tokens request body cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the count-tokens request failed")]
    Transport(#[source] TransportError),
    #[error("Token counting request failed with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the count-tokens reply is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the count-tokens reply is not a JSON object")]
    NotAnObject,
    #[error("input_tokens in the count-tokens reply is not an integer")]
    InputTokens,
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
}

impl PartnerCountError {
    // LiteLLM's wrapper re-raises an exception that carries status_code and answers the rest with a 500
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::UnknownPublisher { .. }
            | Self::MissingMessages
            | Self::VertexUrl(_)
            | Self::Url(_)
            | Self::Header(_)
            | Self::Encode(_)
            | Self::Transport(_)
            | Self::Decode(_)
            | Self::NotAnObject
            | Self::InputTokens
            | Self::Mint(_) => None,
        }
    }
}
