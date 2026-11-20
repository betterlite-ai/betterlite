use std::error::Error as StdError;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::config::{BaseRealtimeHttpConfig, ErrorInfo};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::error::{
    ClientError, FailureFacts, handle_error, header_map,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use super::decode::SdpError;

// The exception provider_config.get_error_class raises, with the synthesized status _handle_error flags on it
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
    pub status_code_is_synthesized: bool,
}

// The masked HTTPStatusError a handler with no provider_config lets escape: its response status, text and headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct StatusFailure {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

// The litellm.Timeout a handler with no provider_config lets escape, already carrying the handler's own message
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct TimeoutFailure {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum RealtimeHttpError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Status(#[from] StatusFailure),
    #[error(transparent)]
    Timeout(#[from] TimeoutFailure),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Sdp(#[from] SdpError),
    #[error(transparent)]
    Config(Box<dyn StdError + Send + Sync>),
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {name} header value is not text")]
    HeaderNotText { name: String },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// _handle_error: the shared converter picks the status, text and headers, and the provider config names the error class
pub(crate) fn provider_failure<C: BaseRealtimeHttpConfig>(
    config: &C,
    facts: &FailureFacts,
) -> ProviderError {
    let converted = handle_error(facts, None);
    let error = config.get_error_class(&ErrorInfo {
        message: &converted.message,
        status_code: converted.status_code,
        headers: &converted.headers,
    });
    ProviderError {
        status_code: error.status_code,
        message: error.message,
        headers: error.headers,
        status_code_is_synthesized: converted.status_code_is_synthesized,
    }
}

// http_handler's raise_for_status hands the handler a masked response: no content-encoding, and a length for the decoded body
pub(crate) fn status_failure(answer: &UpstreamResponse) -> StatusFailure {
    let mut headers = header_map(&answer.headers);
    headers.remove("content-encoding");
    headers.remove("content-length");
    if !answer.body.is_empty() {
        headers.insert(
            String::from("content-length"),
            Json::Text(answer.body.len().to_string()),
        );
    }
    StatusFailure {
        status_code: answer.status,
        message: body_text(&answer.body),
        headers,
    }
}

pub(crate) fn display_facts(display: String) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display,
    }
}

// litellm.Timeout's own message for an async handler, which prefixes the handler's text with its class name
pub(crate) fn timeout_failure(timeout: Duration, elapsed: Duration) -> TimeoutFailure {
    TimeoutFailure {
        message: format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
            timeout_text(timeout),
            elapsed.as_secs(),
            elapsed.subsec_millis()
        ),
    }
}

// The handler interpolates the timeout it ran with, so a whole-second value shows as its integer
fn timeout_text(timeout: Duration) -> String {
    if timeout.subsec_nanos() == 0 {
        return timeout.as_secs().to_string();
    }
    timeout.as_secs_f64().to_string()
}

fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}
