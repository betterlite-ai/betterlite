use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use betterlite_postgres_fixture::error::FixtureError;
use thiserror::Error;

use crate::OracleError;
use crate::multipart_oracle::error::MultipartError;

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("scenario input exceeds {limit} bytes")]
    InputTooLarge { limit: usize },
    #[error("scenario input is not valid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
    #[error("cannot parse scenario: {0}")]
    Parse(#[source] serde_saphyr::Error),
    #[error("scenario field \"{field}\" is not allowed")]
    UnknownField { field: String },
    #[error("duplicate field \"{field}\"")]
    DuplicateField { field: String },
    #[error("scenario kind must be \"http_scenario\"")]
    Kind,
    #[error("scenario {field} cannot be empty")]
    Empty { field: &'static str },
    #[error("scenario env entry \"{name}\" is reserved")]
    ReservedEnv { name: String },
    #[error("scenario must contain between {min} and {max} steps")]
    StepCount { min: usize, max: usize },
    #[error("scenario step {index} {field} cannot be empty")]
    StepField { index: usize, field: &'static str },
    #[error("scenario step {index} capture name \"{name}\" is already declared")]
    DuplicateCapture { index: usize, name: String },
    #[error("scenario step {index} body is invalid: {source}")]
    Body {
        index: usize,
        source: MultipartError,
    },
    #[error("scenario step {index} compare.ignore {problem}")]
    Ignore { index: usize, problem: &'static str },
    #[error("scenario step {index} has a malformed capture placeholder")]
    MalformedPlaceholder { index: usize },
    #[error("scenario step {index} header \"{header}\" cannot use secret capture \"{name}\"")]
    SecretCaptureHeader {
        index: usize,
        header: String,
        name: String,
    },
    #[error("scenario seeds more than {max} rows")]
    TooManyRows { max: usize },
    #[error("scenario fixtures exceed {limit} bytes")]
    FixturesTooLarge { limit: usize },
    #[error("scenario exceeded 120 seconds")]
    Deadline,
    #[error("scenario step {index} capture {name} {reason}")]
    Capture {
        index: usize,
        name: String,
        reason: String,
    },
    #[error("scenario step {index} references unresolved capture \"{name}\"")]
    UnresolvedCapture { index: usize, name: String },
    #[error("{message}")]
    Message { message: String },
    #[error(transparent)]
    Request(Box<OracleError>),
    #[error("scenario fixture failed: {0}")]
    Fixture(#[source] FixtureError),
    #[error("scenario artifact {path} failed: {source}")]
    Artifact { path: PathBuf, source: io::Error },
    #[error("scenario log {path} is not valid UTF-8: {source}")]
    LogUtf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
    #[error("cannot serialize scenario artifact: {0}")]
    Json(#[source] serde_json::Error),
    #[error("scenario I/O failed: {0}")]
    Io(#[source] io::Error),
    #[error("scenario operation failed and cleanup failed: {operation}; {cleanup}")]
    OperationCleanup {
        #[source]
        operation: Box<Self>,
        cleanup: Box<Self>,
    },
}
