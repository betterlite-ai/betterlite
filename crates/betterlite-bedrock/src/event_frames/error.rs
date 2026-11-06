use std::str::Utf8Error;

use aws_smithy_eventstream::error::Error as FrameError;
use base64::DecodeError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorInput, HeaderError};

#[derive(Debug, Error)]
pub enum EventFrameError {
    #[error(
        "event-stream prelude checksum 0x{actual:08X} does not match the computed 0x{expected:08X}"
    )]
    PreludeChecksum { expected: u32, actual: u32 },
    #[error("event-stream frame is malformed")]
    Frame(#[source] FrameError),
    #[error("event-stream header {name} appears twice")]
    DuplicateHeader { name: String },
    #[error("event-stream bytes are not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
    #[error("event-stream payload is not valid JSON")]
    NotJson(#[source] serde_json::Error),
    #[error("event-stream payload is not a JSON object")]
    PayloadNotObject,
    #[error("event-stream chunk bytes are not a base64 string")]
    ChunkBytesNotText,
    #[error("event-stream chunk bytes are not valid base64")]
    ChunkBase64(#[source] DecodeError),
    #[error("bedrock error ({status})")]
    Bedrock {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

impl EventFrameError {
    pub(super) fn upstream(status_code: u16, message: String) -> Self {
        let input = ErrorInput {
            status_code,
            message,
            request: None,
            body: None,
            status_code_is_synthesized: false,
            headers: None,
            response: None,
        };
        match BedrockError::try_from(input) {
            Ok(error) => Self::Bedrock {
                status: status_code,
                error: Box::new(error),
            },
            Err(source) => Self::Header(source),
        }
    }
}
