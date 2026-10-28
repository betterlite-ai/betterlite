use std::io;

use thiserror::Error;

use crate::otlp_normalize::OTLP_MAX_BODY_BYTES;

mod tests;

// LiteLLM raises OTLPPayloadTooLargeError for the TooLarge variants and InvalidOTLPPayloadError for the rest
#[derive(Debug, Error)]
pub enum OtlpError {
    #[error("OTLP body exceeds {OTLP_MAX_BODY_BYTES} bytes")]
    BodyTooLarge,
    #[error("Unsupported OTLP content encoding")]
    UnsupportedEncoding,
    #[error("Invalid OTLP gzip body")]
    InvalidGzip(#[source] io::Error),
    #[error("{0}")]
    SpansTooLarge(String),
    #[error("{0}")]
    InvalidSpans(String),
}

// The native codec raises OverflowError for a payload over its limits and ValueError for one it cannot parse
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("{0}")]
    Overflow(String),
    #[error("{0}")]
    Malformed(String),
}
