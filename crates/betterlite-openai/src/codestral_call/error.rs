use crate::aleph_alpha::error::AlephAlphaError;
use crate::codestral_records::error::TextCompletionCodestralError;
use crate::error::OpenAiError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::error::FactoryError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::ParseError;
pub const TRANSPORT_STATUS: u16 = 500;
#[derive(Debug, Error)]
pub enum CodestralCallError {
    #[error("no Codestral API key is set")]
    MissingApiKey,
    #[error("the Codestral prompt could not be built")]
    Prompt(#[from] FactoryError),
    #[error("the Codestral prompt selection returned no text")]
    PromptNotText,
    #[error("the Codestral settings could not be read")]
    Settings(#[from] OpenAiError),
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Codestral URL is not valid")]
    Endpoint(#[source] ParseError),
    #[error("the Codestral request body could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Record(#[from] TextCompletionCodestralError),
    #[error("the Codestral provider could not be reached")]
    Transport {
        status: u16,
        #[source]
        source: TransportError,
    },
    #[error("the Codestral line stream could not be read")]
    Lines {
        #[source]
        source: AlephAlphaError,
    },
}
impl CodestralCallError {
    #[must_use]
    pub const fn transport(source: TransportError, wrapped: bool) -> Self {
        let timed = matches!(
            &source,
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
        );
        let status = if wrapped || !timed { 500 } else { 408 };
        Self::Transport { status, source }
    }
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Record(TextCompletionCodestralError::Native(native)) => Some(native.status_code),
            Self::Record(_) => Some(TRANSPORT_STATUS),
            Self::Transport { status, .. } => Some(*status),
            Self::MissingApiKey
            | Self::Prompt(_)
            | Self::PromptNotText
            | Self::Settings(_)
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::Endpoint(_)
            | Self::Encode { .. }
            | Self::Lines { .. } => None,
        }
    }
}
