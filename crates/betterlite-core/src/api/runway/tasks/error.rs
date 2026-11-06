use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::api::runway::tasks::TaskKind;

#[derive(Debug, Error)]
pub enum RunwayTaskError {
    #[error(transparent)]
    Environment {
        #[from]
        source: EnvError,
    },
    #[error("RUNWAYML_API_SECRET or RUNWAYML_API_KEY is not set")]
    MissingCredentials,
    #[error("a RunwayML header value holds a character a header cannot carry")]
    InvalidHeader(#[source] InvalidHeaderValue),
    #[error("the RunwayML task URL is not valid")]
    TaskUrl(#[source] url::ParseError),
    #[error(transparent)]
    TaskId(HttpError),
    #[error("{} polling timed out after {seconds} seconds", .kind.polling_subject())]
    TimedOut { kind: TaskKind, seconds: u64 },
    #[error("the RunwayML task poll failed")]
    Transport(#[source] TransportError),
    #[error("the RunwayML task poll answered HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("the RunwayML task answer is not a task: {source}")]
    Decode {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the RunwayML task answer has bytes after the task")]
    TrailingBytes(#[source] serde_json::Error),
    #[error("the RunwayML task answer is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("{} failed: {reason} (code: {code})", .kind.subject())]
    Failed {
        kind: TaskKind,
        reason: String,
        code: String,
    },
    #[error("{} was cancelled", .kind.subject())]
    Cancelled { kind: TaskKind },
    #[error("Unknown RunwayML task status: {status}")]
    UnknownStatus { status: String },
}
