use std::error::Error as StdError;
use std::io;
use std::process::ExitStatus;
use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdkHttpError {
    #[error("sdk_http case is {bytes} bytes, past the {limit} byte limit")]
    CaseTooLarge { bytes: usize, limit: usize },
    #[error("sdk_http result is {bytes} bytes, past the {limit} byte limit")]
    ResultTooLarge { bytes: usize, limit: usize },
    #[error("sdk_http input is not valid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
    #[error("cannot parse sdk_http case: {0}")]
    CaseParse(#[source] Box<serde_saphyr::Error>),
    #[error("cannot parse sdk_http result: {0}")]
    ResultParse(#[source] serde_json::Error),
    #[error("invalid sdk_http case: {0}")]
    Invalid(String),
    #[error("the SDK sent {count} requests, past the limit of {limit}")]
    TooManyRequests { count: usize, limit: usize },
    #[error("request {index} carries a body of {bytes} bytes, past the {limit} byte limit")]
    BodyTooLarge {
        index: usize,
        bytes: usize,
        limit: usize,
    },
    #[error("the execution deadline passed and the work was stopped")]
    Deadline,
    #[error("the execution was cancelled and the work was stopped")]
    Cancelled,
    #[error("cannot start the SDK process: {0}")]
    Spawn(#[source] io::Error),
    #[error("SDK process I/O failed: {0}")]
    Io(#[source] io::Error),
    #[error("SDK process pipe thread panicked")]
    Join,
    #[error("SDK process exited with {status}: {stderr}")]
    Exited { status: ExitStatus, stderr: String },
    #[error("SDK process wrote more than {limit} bytes")]
    OutputTooLarge { limit: usize },
    #[error("cannot serialize the SDK call: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("cannot set up the native service: {0}")]
    Setup(#[source] Box<dyn StdError + Send + Sync>),
    #[error("the native service failed outside a provider answer: {0}")]
    Native(#[source] Box<dyn StdError + Send + Sync>),
    #[error("the native error class {0} has no LiteLLM SDK projection yet")]
    Unprojected(String),
}
