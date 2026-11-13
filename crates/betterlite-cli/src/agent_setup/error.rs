use std::io;
use std::path::PathBuf;

use thiserror::Error;
use tokio::task::JoinError;

use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::receipts::error::ReceiptsError;
use crate::codex_settings::error::CodexSettingsError;
use crate::config::error::ConfigError;
use crate::pi::PiSyncError;

#[derive(Debug, Error)]
pub enum AgentSetupError {
    #[error("{0}")]
    Usage(String),
    #[error(
        "`lite configure` needs a long-lived virtual key: pass --api-key, `lite --api-key`, or set LITELLM_PROXY_API_KEY. Your `lite login` credential expires within a day, so it is not written into agent settings."
    )]
    MissingCredential,
    #[error("The virtual key must not be blank or contain whitespace or control characters.")]
    InvalidCredential,
    #[error("LiteLLM rejected your key (HTTP {status}). Pass a valid --api-key.")]
    ListingRejected {
        status: u16,
        #[source]
        reason: PiSyncError,
    },
    #[error(
        "Could not connect. Is the proxy at {base_url} running, and is --base-url (or LITELLM_PROXY_URL) correct?"
    )]
    ListingUnreachable {
        base_url: String,
        #[source]
        reason: PiSyncError,
    },
    #[error(
        "{reason}. {agent} would have nothing to run; give the key access to at least one model."
    )]
    ListingEmpty {
        agent: &'static str,
        #[source]
        reason: PiSyncError,
    },
    #[error(
        "The proxy at {base_url} answered, so check that it is a LiteLLM proxy and is healthy."
    )]
    ListingFailed {
        base_url: String,
        #[source]
        reason: PiSyncError,
    },
    #[error(
        "The /v1/models listing from {base_url} is too large to load; check that it is a LiteLLM proxy."
    )]
    ListingTooLarge { base_url: String },
    #[error("{model} is not served by {base_url} for this key. /v1/models lists: {listed}.")]
    ModelNotServed {
        model: String,
        base_url: String,
        listed: String,
    },
    #[error("The gateway changed. Pass --api-key for the new gateway; the saved key was not used.")]
    GatewayChanged {
        #[source]
        source: Box<Self>,
    },
    #[error("Invalid gateway setup; no settings were changed")]
    InvalidSetup {
        #[source]
        source: Box<AgentSetupProfilesError>,
    },
    #[error(transparent)]
    Profile(Box<AgentSetupProfilesError>),
    #[error(transparent)]
    ClaudeSettings(Box<ClaudeSettingsError>),
    #[error(transparent)]
    CodexSettings(Box<CodexSettingsError>),
    #[error(transparent)]
    Claude(Box<ReceiptsError>),
    #[error(transparent)]
    Config(Box<ConfigError>),
    #[error("the model listing task did not finish: {0}")]
    ListingTask(#[source] JoinError),
    #[error("the model listing task stopped without an answer")]
    ListingTaskStopped,
    #[error(transparent)]
    Prompt(#[from] inquire::InquireError),
    #[error("could not write the output: {source}")]
    Output {
        #[from]
        source: io::Error,
    },
    #[error("the {target} settings path is not valid UTF-8")]
    SettingsPathNotUtf8 { target: Target },
    #[error("could not resolve the settings path {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot build the Claude Code model pattern: {source}")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("{source} {name} setup was saved. Run `lite configure {target}` to retry applying it")]
    Apply {
        name: &'static str,
        target: Target,
        #[source]
        source: Box<Self>,
    },
}

impl From<AgentSetupProfilesError> for AgentSetupError {
    fn from(source: AgentSetupProfilesError) -> Self {
        Self::Profile(Box::new(source))
    }
}

impl From<ClaudeSettingsError> for AgentSetupError {
    fn from(source: ClaudeSettingsError) -> Self {
        Self::ClaudeSettings(Box::new(source))
    }
}

impl From<CodexSettingsError> for AgentSetupError {
    fn from(source: CodexSettingsError) -> Self {
        Self::CodexSettings(Box::new(source))
    }
}

impl From<ReceiptsError> for AgentSetupError {
    fn from(source: ReceiptsError) -> Self {
        Self::Claude(Box::new(source))
    }
}
