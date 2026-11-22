use std::fmt;

use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::image_models::sizing::SizeError;
use crate::invoke_response::RawInvokeReply;

#[derive(Debug, Error)]
pub enum InvokeReplyError {
    #[error("the response body is not valid JSON")]
    Decode(Box<BedrockError>),
    #[error("the response could not be processed")]
    Processing(Box<BedrockError>),
    #[error("the response text could not be assigned")]
    Content(Box<BedrockError>),
    #[error("the response token count header {name} is not an integer")]
    TokenCountHeader {
        name: &'static str,
        #[source]
        source: SizeError,
    },
    #[error("the usage field {name} is out of range")]
    UsageRange { name: &'static str },
    #[error("the token count failed")]
    TokenCount {
        #[source]
        source: CountError,
    },
    #[error("the response creation time is out of range")]
    CreatedOutOfRange,
    #[error("the injected clock is out of range")]
    ClockOutOfRange,
    #[error("the response header {name} cannot be represented")]
    InvalidHeader { name: String },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

impl InvokeReplyError {
    #[must_use]
    pub fn bedrock(&self) -> Option<&BedrockError> {
        match self {
            Self::Decode(error) | Self::Processing(error) | Self::Content(error) => Some(error),
            Self::TokenCountHeader { .. }
            | Self::UsageRange { .. }
            | Self::TokenCount { .. }
            | Self::CreatedOutOfRange
            | Self::ClockOutOfRange
            | Self::InvalidHeader { .. }
            | Self::Header(_) => None,
        }
    }
}

// LiteLLM raises the bare Exception() for an empty text, so its message ends after the dash.
pub(super) enum ContentReason {
    Empty,
    NoChoice,
    NotText,
}

impl fmt::Display for ContentReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => Ok(()),
            Self::NoChoice => formatter.write_str("list index out of range"),
            Self::NotText => formatter.write_str("the response text is not a string"),
        }
    }
}

pub(super) fn decode_error(raw: &RawInvokeReply<'_>) -> InvokeReplyError {
    let message = replaced_text(raw.body);
    match bedrock_error(raw.status, message, raw.headers) {
        Ok(error) => InvokeReplyError::Decode(Box::new(error)),
        Err(error) => error,
    }
}

pub(super) fn processing_error(
    raw: &RawInvokeReply<'_>,
    source: &dyn fmt::Display,
) -> InvokeReplyError {
    let message = format!(
        "Error processing={}, Received error={source}",
        replaced_text(raw.body)
    );
    match bedrock_error(422_u16, message, raw.headers) {
        Ok(error) => InvokeReplyError::Processing(Box::new(error)),
        Err(error) => error,
    }
}

pub(super) fn content_error(
    raw: &RawInvokeReply<'_>,
    output: &str,
    reason: ContentReason,
) -> InvokeReplyError {
    let message = format!("Error parsing received text={output}.\nError-{reason}");
    match bedrock_error(raw.status, message, raw.headers) {
        Ok(error) => InvokeReplyError::Content(Box::new(error)),
        Err(error) => error,
    }
}

fn bedrock_error(
    status: u16,
    message: String,
    headers: &[(String, String)],
) -> Result<BedrockError, InvokeReplyError> {
    BedrockError::try_from(ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    })
    .map_err(InvokeReplyError::Header)
}

// httpx shows one U+FFFD for each invalid byte sequence, which is what raw_response.text holds.
fn replaced_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
