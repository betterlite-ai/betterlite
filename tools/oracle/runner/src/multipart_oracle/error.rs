use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MultipartError {
    #[error("part must contain exactly one payload")]
    PartPayload,
    #[error("part name must not be empty")]
    PartName,
    #[error(
        "part name and filename cannot hold quotes or line breaks, nor content_type line breaks"
    )]
    PartMetadata,
    #[error("an upstream answer with body_kind must use body_kind: bytes")]
    AnswerKind,
    #[error("field \"{field}\" is not allowed")]
    UnknownField { field: String },
    #[error("body specification is invalid: {0}")]
    Specification(#[source] serde_json::Error),
    #[error("request specification is invalid: {0}")]
    RequestSpecification(#[source] Box<serde_saphyr::Error>),
    #[error("request specification is not valid UTF-8: {0}")]
    RequestUtf8(#[source] Utf8Error),
    #[error("bytes answer content_type {declared} differs from its content-type header {header}")]
    AnswerContentType { declared: String, header: String },
    #[error("fixture path escapes multipart fixture root")]
    FixtureEscape,
    #[error("fixture exceeds {limit} bytes")]
    FixtureTooLarge { limit: usize },
    #[error("cannot read fixture {path}: {source}")]
    FixtureRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("multipart contains more than {limit} parts")]
    TooManyParts { limit: usize },
    #[error("captured body exceeds {limit} bytes")]
    CapturedTooLarge { limit: usize },
    #[error("multipart capture has no valid boundary")]
    Boundary,
    #[error("multipart part header {line} is not supported")]
    PartHeader { line: String },
    #[error("capture input is shorter than its Content-Type prefix")]
    CaptureInput,
}
