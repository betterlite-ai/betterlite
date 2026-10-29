use crate::events::RunResult;
use crate::payloads::PayloadError;
use crate::state::Incompatibility;
use crate::types::{Capability, Harness, Usage};
use std::io;
use thiserror::Error;
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CappedOperation {
    #[error("read")]
    Read,
    #[error("run output")]
    RunOutput,
}
#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("Harness.{} does not support {capability:?}", .harness.name())]
    CapabilityUnsupported {
        harness: Harness,
        capability: Capability,
    },
    #[error("{given} cannot be used with Harness.{}; use {expected}", .harness.name())]
    OptionsMismatch {
        given: &'static str,
        harness: Harness,
        expected: &'static str,
    },
    #[error("the runtime is missing from the sandbox or failed to start: {0}")]
    InstallFailed(#[source] io::Error),
    #[error("the sandbox failed: {source}")]
    Sandbox {
        #[source]
        source: io::Error,
    },
    #[error("a turn was started on a session that is closed or detached")]
    SessionClosed,
    #[error("{source}")]
    StateIncompatible { source: Incompatibility },
    #[error("the harness state could not be encoded")]
    StateEncode { source: serde_json::Error },
    #[error("the final answer did not validate against the output schema: {message}")]
    OutputInvalid {
        message: String,
        raw: String,
        result: Option<Box<RunResult>>,
    },
    #[error("the runtime reported a failed turn: {reason}")]
    TurnFailed { reason: String },
    #[error("harness must be a Harness member, got the string {given:?}")]
    NotAHarness { given: String },
    #[error("harness must be a Harness member, got the string {given:?}; did you mean Harness.{}?", .hint.name())]
    HarnessHint { given: String, hint: Harness },
    #[error("the token counts overflow: {usage:?}")]
    TokenOverflow { usage: Usage },
    #[error("sandbox {operation} exceeded the {limit}-byte content limit")]
    ContentLimitExceeded {
        operation: CappedOperation,
        limit: usize,
    },
    #[error("the output schema could not be prepared: {source}")]
    OutputSchema {
        #[source]
        source: PayloadError,
    },
}
