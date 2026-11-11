use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("the gateway executable was not found on PATH; install it with `{install_step}`")]
    GatewayMissing { install_step: &'static str },
    #[error("{path} does not run `--version`; install it with `{install_step}`")]
    GatewayNotRunnable {
        path: PathBuf,
        install_step: &'static str,
    },
    #[error("cannot read the PID record {path}: {source}")]
    ReadPidRecord {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} contains invalid or unexpected JSON; cannot proceed safely.")]
    CorruptPidRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot encode the PID record for {path}: {source}")]
    EncodePidRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot write the PID record {path}: {source}")]
    WritePidRecord {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot clear the PID record {path}: {source}")]
    ClearPidRecord {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot open the log file {path}: {source}")]
    LogFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start {program}: {source}")]
    Launch {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot inspect the started gateway process: {source}")]
    ProcessStatus {
        #[source]
        source: io::Error,
    },
    #[error("Ephemeral proxy exited early (code {code}). Last log lines:\n{tail}")]
    ProxyExitedEarly { code: String, tail: String },
    #[error("Ephemeral proxy never became healthy within {timeout}s. Last log lines:\n{tail}")]
    ProxyNeverHealthy { timeout: u64, tail: String },
    #[error("the log-tailing thread could not start: {source}")]
    LogThread {
        #[source]
        source: io::Error,
    },
}
