use std::str::Utf8Error;

use thiserror::Error;

use crate::agentcore_response::error::BedrockFailure;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

#[derive(Debug, Error)]
pub enum SearchReplyError {
    #[error("the AgentCore search reply is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the AgentCore search reply is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the AgentCore search reply is not a JSON object")]
    NotObject,
    #[error("the AgentCore search reply could not be re-encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("bedrock provider error ({status})")]
    Provider {
        status: u16,
        #[source]
        cause: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

impl SearchReplyError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => Some(*status),
            Self::NotUtf8 { .. }
            | Self::Json { .. }
            | Self::NotObject
            | Self::Encode { .. }
            | Self::Header(_) => None,
        }
    }
}

pub(crate) fn provider_error(
    status: u16,
    message: String,
    headers: &[(String, String)],
) -> SearchReplyError {
    let input = ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => SearchReplyError::Provider {
            status,
            cause: BedrockFailure {
                cause: Box::new(error),
            },
        },
        Err(source) => SearchReplyError::Header(source),
    }
}
