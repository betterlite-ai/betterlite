use std::io;

use thiserror::Error;

use crate::claude_settings::error::ClaudeSettingsError;
use crate::credentials::login::LoginError as PkceError;
use crate::session::error::SessionError;
use crate::sso_polling::error::PollingError;

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("Cannot configure Claude Code, so not logging in: {source}")]
    NotLoggingIn {
        #[source]
        source: PreflightError,
    },
    #[error("Logged in, but could not configure Claude Code: {source}")]
    ConfigureClaude {
        #[source]
        source: SessionError,
    },
    #[error("could not write to the terminal: {source}")]
    Write {
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error(transparent)]
    Settings(#[from] ClaudeSettingsError),
    #[error("cannot find your home directory")]
    NoHome,
}

#[derive(Debug, Error)]
pub enum FlowError {
    #[error(transparent)]
    Pkce(#[from] PkceError),
    #[error(transparent)]
    Polling(#[from] PollingError),
    #[error("could not write to the terminal: {0}")]
    Write(#[from] io::Error),
    #[error("the async runtime could not start: {0}")]
    Runtime(#[source] io::Error),
    #[error("Logged in, but could not configure Claude Code: {0}")]
    Configure(#[source] SessionError),
}
