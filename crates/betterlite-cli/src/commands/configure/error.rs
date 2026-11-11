use std::io;

use thiserror::Error;

use crate::agent_setup::error::AgentSetupError;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::receipts::error::ReceiptsError;
use crate::codex_settings::error::CodexSettingsError;
use crate::config::error::ConfigError;

#[derive(Debug, Error)]
pub enum ConfigureCommandError {
    #[error("{0}")]
    Usage(String),
    #[error(
        "`lite {command}` asks questions, so it needs a terminal. Non-interactively, run `lite {command} claude --api-key <key> --model <model>` or `lite {command} codex --api-key <key> --model <model>`"
    )]
    Terminal { command: &'static str },
    #[error(transparent)]
    Setup(Box<AgentSetupError>),
    #[error(transparent)]
    Profiles(Box<AgentSetupProfilesError>),
    #[error(transparent)]
    ClaudeSettings(Box<ClaudeSettingsError>),
    #[error(transparent)]
    Claude(Box<ReceiptsError>),
    #[error(transparent)]
    Codex(Box<CodexSettingsError>),
    #[error(transparent)]
    Config(Box<ConfigError>),
    #[error("could not write the output: {source}")]
    Output {
        #[from]
        source: io::Error,
    },
}

impl From<AgentSetupError> for ConfigureCommandError {
    fn from(source: AgentSetupError) -> Self {
        match source {
            AgentSetupError::Usage(text) => Self::Usage(text),
            other @ (AgentSetupError::MissingCredential
            | AgentSetupError::InvalidCredential
            | AgentSetupError::ListingRejected { .. }
            | AgentSetupError::ListingUnreachable { .. }
            | AgentSetupError::ListingEmpty { .. }
            | AgentSetupError::ListingFailed { .. }
            | AgentSetupError::ListingTooLarge { .. }
            | AgentSetupError::ModelNotServed { .. }
            | AgentSetupError::GatewayChanged { .. }
            | AgentSetupError::InvalidSetup { .. }
            | AgentSetupError::Profile(_)
            | AgentSetupError::ClaudeSettings(_)
            | AgentSetupError::CodexSettings(_)
            | AgentSetupError::Claude(_)
            | AgentSetupError::Config(_)
            | AgentSetupError::ListingTask(_)
            | AgentSetupError::ListingTaskStopped
            | AgentSetupError::Prompt(_)
            | AgentSetupError::Output { .. }
            | AgentSetupError::SettingsPathNotUtf8 { .. }
            | AgentSetupError::Resolve { .. }
            | AgentSetupError::Pattern { .. }
            | AgentSetupError::Apply { .. }) => Self::Setup(Box::new(other)),
        }
    }
}

impl From<AgentSetupProfilesError> for ConfigureCommandError {
    fn from(source: AgentSetupProfilesError) -> Self {
        Self::Profiles(Box::new(source))
    }
}

impl From<ClaudeSettingsError> for ConfigureCommandError {
    fn from(source: ClaudeSettingsError) -> Self {
        Self::ClaudeSettings(Box::new(source))
    }
}

impl From<ReceiptsError> for ConfigureCommandError {
    fn from(source: ReceiptsError) -> Self {
        Self::Claude(Box::new(source))
    }
}

impl From<CodexSettingsError> for ConfigureCommandError {
    fn from(source: CodexSettingsError) -> Self {
        Self::Codex(Box::new(source))
    }
}

impl From<ConfigError> for ConfigureCommandError {
    fn from(source: ConfigError) -> Self {
        match source {
            ConfigError::Usage(text) => Self::Usage(text),
            other @ (ConfigError::KeyNotSet(_)
            | ConfigError::NoHome
            | ConfigError::Runtime { .. }
            | ConfigError::Save(_)) => Self::Config(Box::new(other)),
        }
    }
}
