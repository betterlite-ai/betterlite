use crate::agent_launch::AgentRunError as LaunchError;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::pi::PiSyncError;
use std::io;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum AgentRunError {
    #[error("Nothing to run.")]
    NothingToRun,
    #[error("Could not find `{command}` on your PATH.{hint}")]
    MissingBinary { command: String, hint: String },
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    Settings(#[from] ClaudeSettingsError),
    #[error(transparent)]
    Pi(#[from] PiSyncError),
    #[error("could not restore the controlling terminal: {source}")]
    Terminal {
        #[source]
        source: io::Error,
    },
}
