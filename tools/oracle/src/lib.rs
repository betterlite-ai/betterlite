#![expect(
    clippy::disallowed_types,
    reason = "The oracle compares arbitrary provider JSON, so captured payloads require serde_json::Value."
)]

mod artifacts;
pub mod case;
mod cli;
mod compare;
mod gateway;
#[expect(
    clippy::mod_module_files,
    clippy::disallowed_methods,
    clippy::absolute_paths,
    clippy::map_err_ignore,
    reason = "The bounded helper process runner owns these CLI and pipe operations."
)]
mod helpers;
pub mod http;
pub mod mock;
mod paths;
pub mod process;
pub mod routing_mock_scripts;
pub mod routing_probe_ipc;
pub mod runner;

mod tests;

use std::io;
use std::path::PathBuf;
use std::process::{ExitCode, ExitStatus};
use std::str::Utf8Error;
use std::time::{Duration, SystemTimeError};

use axum::http::status::InvalidStatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OracleError {
    #[error(transparent)]
    Cli(#[from] clap::Error),
    #[error("cannot read case {path}: {source}")]
    CaseRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot parse case {path}: {source}")]
    CaseParse {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("invalid case {path}: {message}")]
    CaseInvalid { path: PathBuf, message: String },
    #[error("HTTP request failed: {0}")]
    HttpRequest(#[source] reqwest::Error),
    #[error("HTTP I/O failed: {0}")]
    HttpIo(#[source] io::Error),
    #[error("HTTP response exceeded 16 MiB")]
    HttpTooLarge,
    #[error("response body is not valid UTF-8: {0}")]
    HttpText(#[source] Utf8Error),
    #[error("response body event is not valid JSON: {0}")]
    HttpBody(#[source] serde_json::Error),
    #[error("mock provider failed: {0}")]
    MockIo(#[source] io::Error),
    #[error("mock provider thread panicked")]
    MockJoin,
    #[error("mock script is {bytes} bytes, past the 16 MiB limit")]
    ScriptTooLarge { bytes: usize },
    #[error("cannot parse mock script: {0}")]
    ScriptParse(#[source] serde_json::Error),
    #[error("mock script has {count} replies, past the limit of 256")]
    ScriptReplies { count: usize },
    #[error("mock script deadline_ms {millis} is outside 1 to 30000")]
    ScriptDeadline { millis: u64 },
    #[error("mock script reply {index} waits {millis} ms, past the script deadline")]
    ScriptDelay { index: usize, millis: u64 },
    #[error("mock script reply {index} must contain exactly one of body or events")]
    ScriptAnswer { index: usize },
    #[error("mock script reply {index} has an invalid status: {source}")]
    ScriptStatus {
        index: usize,
        #[source]
        source: InvalidStatusCode,
    },
    #[error("mock script barrier {name} is held by fewer than two replies")]
    ScriptBarrier { name: String },
    #[error("cannot open gateway log {path}: {source}")]
    ProcessLog {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start gateway: {0}")]
    ProcessSpawn(#[source] io::Error),
    #[error("cannot synchronize LiteLLM environment: {0}")]
    ProcessSync(#[source] io::Error),
    #[error("LiteLLM environment synchronization exited with {0}")]
    ProcessSyncExited(ExitStatus),
    #[error("gateway exited before readiness: {0}")]
    ProcessExited(ExitStatus),
    #[error("gateway did not become ready within 120 seconds")]
    ProcessTimeout,
    #[error("gateway request failed while waiting for readiness: {source}")]
    ProcessReady {
        #[source]
        source: Box<Self>,
    },
    #[error("cannot stop gateway: {0}")]
    ProcessStop(#[source] io::Error),
    #[error("gateway did not return a response for case {0}")]
    MissingCapture(usize),
    #[error("cannot write progress: {0}")]
    Output(#[source] io::Error),
    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot serialize artifact: {0}")]
    Json(#[source] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(#[source] SystemTimeError),
    #[error("probe message exceeds the 1 MiB limit")]
    ProbeTooLarge,
    #[error("probe message is malformed: {0}")]
    ProbeMalformed(#[source] serde_json::Error),
    #[error("random sample {0} is not below 2^53")]
    ProbeSample(u64),
    #[error("cannot start probe: {0}")]
    ProbeSpawn(#[source] io::Error),
    #[error("probe was started without piped standard streams")]
    ProbePipe,
    #[error("probe I/O failed: {0}")]
    ProbeIo(#[source] io::Error),
    #[error("probe did not finish within {0:?}")]
    ProbeTimeout(Duration),
    #[error("probe exited with {status}")]
    ProbeExited { status: ExitStatus, stderr: Vec<u8> },
    #[error("helper input is invalid: {message}")]
    HelperInput { message: String },
    #[error("cannot start helper: {0}")]
    HelperSpawn(#[source] io::Error),
    #[error("cannot write helper input: {0}")]
    HelperIo(#[source] io::Error),
    #[error("helper stdin is unavailable")]
    HelperStdin,
    #[error("helper stdout is unavailable")]
    HelperStdout,
    #[error("helper stderr is unavailable")]
    HelperStderr,
    #[error("helper reader thread panicked")]
    HelperJoin,
    #[error("helper timed out and was reaped")]
    HelperTimeout,
    #[error("helper output exceeded 1 MiB")]
    HelperOutputTooLarge,
    #[error("helper exited with {status}: {diagnostic}")]
    HelperExited {
        status: ExitStatus,
        diagnostic: String,
    },
    #[error("helper output is not valid JSON: {0}")]
    HelperJson(#[source] serde_json::Error),
}

pub fn run() -> Result<ExitCode, OracleError> {
    let options = <cli::Options as clap::Parser>::try_parse()?;
    let cases = case::load_cases(&options.paths)?;
    runner::run_cases(&options, cases)
}
