use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::commands::ClientError;
use crate::settings_backup::error::BackupError;

#[derive(Debug, Error)]
pub enum DebugError {
    #[error("the environment has no home directory")]
    NoHome,
    #[error(
        "Could not find a Claude Code session. Pass --session-id or set $CLAUDE_CODE_SESSION_ID."
    )]
    NoSession,
    #[error("cannot list the Claude Code transcripts in {path}")]
    TranscriptScan {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("GET {uri} failed")]
    Request {
        uri: String,
        #[source]
        source: ClientError,
    },
    #[error("GET {uri} failed with HTTP {status}: {body}")]
    Status {
        uri: String,
        status: u16,
        body: String,
    },
    #[error("Unexpected {uri} response")]
    UnexpectedShape {
        uri: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "No spend logs found for session '{session_id}' on {base_url}. Is Claude Code routed through this proxy (`lite up`), and does your key have log access?"
    )]
    NoSessionRows {
        session_id: String,
        base_url: String,
    },
    #[error("the total spend of the session does not fit a decimal")]
    SpendOverflow,
    #[error("the session id '{session_id}' cannot be part of a file name")]
    SessionIdNotFileName { session_id: String },
    #[error("cannot create the directory {path}")]
    ReportDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot create the report file {path}")]
    ReportCreate {
        path: PathBuf,
        #[source]
        source: BackupError,
    },
    #[error("cannot write the report to {path}")]
    ReportWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot create the directory {path}")]
    SlashCommandDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot write the slash command to {path}")]
    SlashCommandWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot write to the terminal")]
    Output(#[from] io::Error),
}

mod tests;
