use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PassthroughStreamError {
    #[error("AsyncPassthroughStream must be awaited before accessing {field}")]
    NotInitialized { field: &'static str },
    #[error("the async passthrough stream cannot be awaited twice; its send was already consumed")]
    AlreadyAwaited,
    #[error("Expected coroutine from async client")]
    ExpectedCoroutine,
    #[error("the upstream answered with status {}", .0.status_code)]
    UpstreamStatus(Box<UpstreamStatus>),
    #[error("the upstream passthrough stream could not be read")]
    StreamRead(#[source] Box<HttpError>),
    #[error("the upstream passthrough response body could not be read")]
    BodyRead(#[source] Box<HttpError>),
    #[error("the passthrough request could not be sent")]
    Send(#[source] Box<TransportError>),
}

#[derive(Debug)]
pub struct UpstreamStatus {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
