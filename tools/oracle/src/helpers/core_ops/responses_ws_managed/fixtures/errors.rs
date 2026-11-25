use super::null_shape::ShapeError;
use betterlite_core::api::response_id_traversal::response_id::ResponseRewriteError;
use betterlite_core::api::responses_create_call::error::AsyncCreateError;
use betterlite_core::api::responses_stream_driver::errors::TransportFailure;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::response_events::event_decode::ResponseEventDecodeError;
use betterlite_redact::error::RedactionError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum FixtureError {
    #[error("the fixture clock is out of range")]
    Clock,
    #[error("the fixture create call cannot be read")]
    Call(#[from] ResponseRewriteError),
    #[error("the fixture logging state cannot be built")]
    Logging(#[from] LoggingStateError),
    #[error("the fixture redactor cannot be built")]
    Redactor(#[from] RedactionError),
    #[error("the fixture environment cannot be read")]
    Proxy(#[from] EnvError),
    #[error("the fixture HTTP client cannot be built")]
    Client(#[from] HttpError),
    #[error("the fixture JSON cannot be read")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub(crate) enum DispatchFailure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error(transparent)]
    Create(#[from] AsyncCreateError),
}

#[derive(Debug, Error)]
pub(crate) enum AdapterError {
    #[error(transparent)]
    Scripted(#[from] ScriptedFailure),
    #[error("{0}")]
    Dispatch(#[source] Box<DispatchFailure>),
    #[error("{0}")]
    Transport(#[source] TransportFailure),
    #[error("{message}")]
    Decode { message: String },
    #[error("'async for' requires an object with __aiter__ method, got ResponsesAPIResponse")]
    NotStreaming,
    #[error("the provider event cannot be decoded")]
    Event(#[from] ResponseEventDecodeError),
    #[error("the provider response id cannot be rewritten")]
    Rewrite(#[from] ResponseRewriteError),
    #[error("the provider event fields cannot be inspected")]
    Shape(#[from] ShapeError),
}

// A scripted provider, stream or socket failure carries only the text the case scripts for it.
#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct ScriptedFailure {
    pub(crate) message: String,
}
