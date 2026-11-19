use betterlite_anthropic::anthropic_http_stream::error::{StreamFailure, StreamOpenError};
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::headers::HeadersError;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_prompts::anthropic_invokes::file_blocks::FileBlockError;
use http::HeaderMap;

use crate::anthropic_dispatch::DispatchError;
use crate::anthropic_dispatch::native::Failure;
use crate::operation::{AnthropicCallError, OperationError};

type CallFailure = Failure<StreamFailure, FileBlockError>;

pub(crate) fn dispatch(error: DispatchError<CallFailure>) -> OperationError {
    match error {
        DispatchError::Completion(source) => failure(source),
        other => internal(other.to_string()),
    }
}

fn failure(source: CallFailure) -> OperationError {
    match source {
        Failure::Http(source) => source,
        Failure::Chat(source) => chat(*source),
        Failure::Prepare(source) => prepare(*source),
        Failure::Config
        | Failure::MissingBase
        | Failure::ExtraBody
        | Failure::Legacy(_)
        | Failure::Json(_)
        | Failure::Url(_)
        | Failure::HeaderName(_)
        | Failure::HeaderValue(_) => internal(source.to_string()),
    }
}

fn chat(source: AnthropicCallError) -> OperationError {
    if timeout(&source) {
        return OperationError::Timeout;
    }
    if matches!(
        source,
        ChatCallError::Headers(HeadersError::Authentication { .. })
    ) {
        return OperationError::MissingApiKey;
    }
    match source.status_code() {
        Some(status) => OperationError::Provider {
            response: Box::new(UpstreamResponse {
                status,
                headers: headers(&source),
                body: source.to_string().into_bytes(),
            }),
        },
        None => OperationError::Anthropic {
            source: Box::new(source),
        },
    }
}

fn prepare(source: RequestError<FileBlockError>) -> OperationError {
    match source.status_code() {
        Some(status) => OperationError::Provider {
            response: Box::new(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: source.to_string().into_bytes(),
            }),
        },
        None => internal(source.to_string()),
    }
}

// LiteLLM's stream handler reports a read that timed out as the request timeout, not as a provider answer.
fn timeout(source: &AnthropicCallError) -> bool {
    let ChatCallError::Open(
        StreamOpenError::Reraised { source } | StreamOpenError::Unknown { source },
    ) = source
    else {
        return false;
    };
    matches!(
        source,
        StreamFailure::Transport(TransportError::ConnectTimeout(_))
            | StreamFailure::Transport(TransportError::Deadline(_))
    )
}

fn headers(source: &AnthropicCallError) -> HeaderMap {
    match source {
        ChatCallError::Response(ResponseError::Anthropic { headers, .. }) => headers.clone(),
        ChatCallError::Open(StreamOpenError::Anthropic { headers, .. })
        | ChatCallError::Anthropic { headers, .. } => headers.clone().unwrap_or_default(),
        ChatCallError::Headers(_)
        | ChatCallError::Prepare { .. }
        | ChatCallError::Filter { .. }
        | ChatCallError::PreCall(_)
        | ChatCallError::Response(_)
        | ChatCallError::Parameter(_)
        | ChatCallError::Open(_)
        | ChatCallError::Endpoint(_)
        | ChatCallError::HeaderName(_)
        | ChatCallError::HeaderValue(_) => HeaderMap::new(),
    }
}

pub(crate) fn logging(source: LoggingStateError) -> OperationError {
    internal(source.to_string())
}

// The 500 AnthropicError LiteLLM raises for a failure its handler did not classify.
fn internal(message: String) -> OperationError {
    OperationError::Anthropic {
        source: Box::new(AnthropicCallError::Anthropic {
            status_code: 500,
            message,
            headers: None,
        }),
    }
}

#[must_use]
pub(crate) fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("anthropic"),
        operation: String::from("provider endpoint"),
    }
}

#[must_use]
pub(crate) fn unavailable() -> OperationError {
    OperationError::UnavailableAdapter {
        provider: String::from("anthropic"),
    }
}
