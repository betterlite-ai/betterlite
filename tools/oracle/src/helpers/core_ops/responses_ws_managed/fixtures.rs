use betterlite_core::api::response_id_traversal::response_id::ResponseRewriteError;
use betterlite_core::api::responses_create_call::error::AsyncCreateError;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use thiserror::Error;

mod answers;
mod config;
mod dispatch;
mod logging;
mod ports;
mod rows;

pub(super) use answers::{requests, start_mock};
pub(super) use config::Configs;
pub(super) use dispatch::dispatch;
pub(super) use logging::LoggingProbe;
pub(super) use ports::{DispatchResolver, NoBridge, NoInterception, Nothing, Outbound, outbound};
pub(super) use rows::{ChunkKind, QuotaKind, ScriptedFailure};

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error("the fixture clock is out of range")]
    Clock,
    #[error("the fixture create call cannot be read")]
    Call(#[from] ResponseRewriteError),
    #[error("the fixture logging state cannot be built")]
    Logging(#[from] LoggingStateError),
    #[error("the fixture redactor cannot be built")]
    Redactor(#[from] RedactionError),
    #[error("the fixture environment cannot be read: {message}")]
    Proxy { message: String },
    #[error("the fixture HTTP client cannot be built")]
    Client(#[from] HttpError),
    #[error("the fixture JSON cannot be read")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub(super) enum DispatchFailure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error(transparent)]
    Create(#[from] AsyncCreateError),
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct HookFailure {
    pub(super) message: String,
}
