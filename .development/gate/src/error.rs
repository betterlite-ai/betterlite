use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use thiserror::Error;
use toml::de;

#[derive(Debug, Error)]
pub(crate) enum GateError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("{path} does not match its format: {source}")]
    Yaml {
        path: PathBuf,
        source: Box<serde_saphyr::Error>,
    },
    #[error("{path} is not valid TOML: {source}")]
    Toml {
        path: PathBuf,
        source: Box<de::Error>,
    },
    #[error("{what} does not match its format: {source}")]
    Json {
        what: String,
        source: serde_json::Error,
    },
    #[error("{what} printed bytes that are not UTF-8: {source}")]
    Utf8 { what: String, source: FromUtf8Error },
    #[error("cannot run {program}: {source}")]
    Spawn { program: String, source: io::Error },
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error("{0} is set but not valid UTF-8")]
    NotUnicode(&'static str),
    #[error("no base commit: set GATE_BASE or create a main branch")]
    NoBase,
    #[error("{0} is not in .development/backlog.yaml")]
    UnknownTask(String),
    #[error("{0} has no task file yet")]
    NoTaskFile(String),
    #[error("{0} is not an optional check in .development/checks.yaml")]
    UnknownCheck(String),
    #[error("invalid glob {pattern}: {source}")]
    Glob {
        pattern: String,
        source: Box<globset::Error>,
    },
    #[error("cannot dispatch {task}: {reason}")]
    Dispatch { task: String, reason: String },
}
