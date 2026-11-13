use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::agent_dispatch::error::AgentDispatchError;
use crate::agent_launch::AgentRunError;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::login::error::LoginError;
use crate::settings_backup::error::BackupError;

#[derive(Debug, Error)]
pub enum UpError {
    #[error("the environment has no home directory")]
    NoHome,
    #[error("No fresh LiteLLM login found for this proxy. Run `{command}` first.")]
    NoFreshLogin { command: &'static str },
    #[error("Login did not produce a usable token.")]
    LoginProducedNoToken,
    #[error(transparent)]
    Login(#[from] AgentDispatchError),
    #[error(transparent)]
    LoginFlow(#[from] LoginError),
    #[error(transparent)]
    Backup(#[from] BackupError),
    #[error(transparent)]
    Settings(#[from] ClaudeSettingsError),
    #[error(
        "{path} already exists -- `lite up` looks like it's already running (or crashed without cleanup). Run `lite down` first."
    )]
    BackupExists { path: PathBuf },
    #[error(
        "LiteLLM rejected your key (HTTP {status}). Run `lite login` to refresh it, or pass a valid --api-key."
    )]
    RejectedKey { status: u16 },
    #[error(
        "Could not reach the LiteLLM proxy at {base}: {source}. Is it running, and is --base-url (or LITELLM_PROXY_URL) correct?"
    )]
    Unreachable {
        base: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Verify(#[from] AgentRunError),
    #[error("cannot create the directory for {path}: {source}")]
    SettingsDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the async runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error("could not write to the terminal: {source}")]
    Write {
        #[source]
        source: io::Error,
    },
    #[error("cannot watch for SIGINT and SIGTERM: {source}")]
    Signals {
        #[source]
        source: io::Error,
    },
}
