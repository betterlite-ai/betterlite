use std::io;
use std::sync::Arc;

use rmcp::ServiceError;
use rmcp::model::ErrorData;
use thiserror::Error;

use crate::client::http_transport::error::McpHttpError;
use crate::client::sessions::stream::{
    MessageHandler, StreamFailure, StreamFailureKind, StreamMessage,
};

#[derive(Debug, Error)]
pub enum SdkError {
    #[error("the MCP HTTP request failed")]
    Http(#[source] Arc<McpHttpError>),
    #[error("SSE stream failed: {0}")]
    Sse(#[source] sse_stream::Error),
    #[error("SSE stream failed: Server-sent event exceeded the {limit} byte limit.")]
    EventTooLarge { limit: usize },
    #[error("the message is not valid JSON-RPC")]
    Json(#[source] serde_json::Error),
    #[error("Endpoint origin does not match connection origin: {endpoint_url}")]
    OriginMismatch { endpoint_url: String },
    #[error("{message}")]
    Redirect { message: String },
    #[error("the stdio server failed")]
    Io(#[source] Arc<io::Error>),
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error("the MCP server answered with a JSON-RPC error")]
    JsonRpc(#[source] ErrorData),
}

impl From<McpHttpError> for SdkError {
    fn from(source: McpHttpError) -> Self {
        Self::Http(Arc::new(source))
    }
}

impl From<io::Error> for SdkError {
    fn from(source: io::Error) -> Self {
        Self::Io(Arc::new(source))
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json(source)
    }
}

// B4744 records the SDK runtime's exception kinds, so the transport trades its variant for that kind
pub(super) fn report_failure(
    handler: &MessageHandler<SdkError>,
    kind: StreamFailureKind,
    error: SdkError,
) {
    let failure = match kind {
        StreamFailureKind::ValueError => StreamFailure::ValueError(Arc::new(error)),
        StreamFailureKind::Http => StreamFailure::Http(Arc::new(error)),
        StreamFailureKind::Io => StreamFailure::Io(Arc::new(error)),
    };
    let _closed = handler.handle(StreamMessage::Failure(failure));
}
