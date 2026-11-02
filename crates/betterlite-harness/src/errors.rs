use crate::events::RunResult;
use crate::payloads::{DEFAULT_MAX_RECURSE_DEPTH, PayloadError};
use crate::state::Incompatibility;
use crate::types::{Capability, Harness, Usage};
use std::io;
use thiserror::Error;

mod python_repr;

use python_repr::str_repr;

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
    #[error("Invalid CodexOptions.config key: {}", str_repr(key))]
    InvalidConfigKey { key: String },
    #[error(
        "CodexOptions.config[{}] is managed by LiteLLM; use the matching agent() argument instead",
        str_repr(key)
    )]
    ManagedConfigKey { key: String },
    #[error("CodexOptions.config value of type NoneType cannot be passed to codex")]
    CodexNullValue,
    #[error("CodexOptions.config is nested deeper than {DEFAULT_MAX_RECURSE_DEPTH} levels")]
    CodexNestedTooDeep,
    #[error(
        "OpenCodeOptions.config[{}] is managed by LiteLLM; use the matching agent() argument (model=, permissions=, disable_tools=) instead",
        str_repr(key)
    )]
    OpenCodeManagedTopLevelKey { key: String },
    #[error(
        "OpenCodeOptions.config[{}][{}] sets [{}], which LiteLLM manages; use permissions=/disable_tools=/model= instead",
        str_repr(section),
        str_repr(agent),
        keys.iter().map(|key| str_repr(key)).collect::<Vec<_>>().join(", ")
    )]
    OpenCodeAgentManagedKeys {
        section: String,
        agent: String,
        keys: Vec<String>,
    },
    #[error("OpenCodeOptions.config[{}] must be a mapping", str_repr(section))]
    OpenCodeSectionNotMapping { section: String },
    #[error(
        "OpenCodeOptions.config[{}][{}] must be a mapping",
        str_repr(section),
        str_repr(agent)
    )]
    OpenCodeAgentShapeInvalid { section: String, agent: String },
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
    #[error(
        "ClaudeCodeOptions.config may not set {keys}; use the matching agent() argument (model=, permissions=) instead"
    )]
    ClaudeCodeManagedConfig { keys: String },
    #[error("ClaudeCodeOptions.env may not set {keys}; LiteLLM manages it")]
    ClaudeCodeManagedEnv { keys: String },
    #[error("Claude Code needs the session model endpoint")]
    ClaudeCodeEndpoint,
}
