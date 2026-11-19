use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::string::FromUtf8Error;

use axum::http::status::InvalidStatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HelperRunError {
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
    #[error("cannot synchronize LiteLLM environment: {0}")]
    ProcessSync(#[source] io::Error),
    #[error("LiteLLM environment synchronization exited with {0}")]
    ProcessSyncExited(ExitStatus),
    #[error("Prisma needs {path}, which is missing")]
    PrismaMissing { path: PathBuf },
    #[error("cannot run prisma generate: {0}")]
    PrismaGenerate(#[source] io::Error),
    #[error("prisma generate exited with {0}")]
    PrismaGenerateExited(ExitStatus),
    #[error("cannot run prisma db execute: {0}")]
    PrismaExecute(#[source] io::Error),
    #[error("prisma db execute exited with {0}")]
    PrismaExecuteExited(ExitStatus),
    #[error("prisma db execute stdin is unavailable")]
    PrismaStdin,
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
    #[error("helper exited with {status} and wrote stderr that is not UTF-8")]
    HelperDiagnostic {
        status: ExitStatus,
        source: FromUtf8Error,
    },
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
    #[error("BETTERLITE_ORACLE_CACHE must be own or shared")]
    CacheMode,
    #[error("HOME is not set, so the shared LiteLLM answer cache has no folder")]
    CacheHome,
    #[error("helper file name is not UTF-8 at {path}")]
    HelperFileName { path: PathBuf },
}
