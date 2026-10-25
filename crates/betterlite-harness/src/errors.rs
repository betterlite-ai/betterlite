use thiserror::Error;

use crate::types::Result;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("capability unsupported: {message}")]
    CapabilityUnsupported { message: String },
    #[error("{actual} cannot be used with Harness.{harness:?}; use {expected}")]
    OptionsMismatch {
        actual: String,
        expected: String,
        harness: String,
    },
    #[error("harness installation failed: {message}")]
    HarnessInstallFailed { message: String },
    #[error("sandbox error: {message}")]
    SandboxError { message: String },
    #[error("session closed: {message}")]
    SessionClosed { message: String },
    #[error("unreadable harness state: {source}")]
    StateIncompatible {
        #[source]
        source: serde_json::Error,
    },
    #[error("state version {found} is not supported (expected 1)")]
    StateVersion { found: u32 },
    #[error("output is invalid: {message}")]
    OutputInvalid {
        message: String,
        raw: String,
        result: Option<Result>,
    },
    #[error("harness turn failed: {message}")]
    TurnFailed { message: String },
}
