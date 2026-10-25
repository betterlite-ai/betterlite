use thiserror::Error;

use crate::events::RunResult;
use crate::state::STATE_VERSION;
use crate::types::{Capability, Harness, Usage};

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
    #[error("the runtime is missing from the sandbox or failed to start: {reason}")]
    InstallFailed { reason: String },
    #[error("the sandbox failed: {reason}")]
    Sandbox { reason: String },
    #[error("a turn was started on a session that is closed or detached")]
    SessionClosed,
    #[error("the harness state is unreadable")]
    StateIncompatible { source: serde_json::Error },
    #[error("harness state version {found} is not supported, expected {STATE_VERSION}")]
    StateVersion { found: u32 },
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
}
