use std::str::from_utf8;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, HeaderMap, ResponseFacts, header_map,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_subtitles::timing::SubtitleTimingError;

use crate::api::transcription_contract::TranscriptionContractError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Number;
use thiserror::Error;

use super::config::ConfigFailure;
use super::wire::ClientKind;

#[derive(Debug, Error)]
pub enum TranscriptionHttpError {
    #[error("No provider config found for model: {model} and provider: {provider}")]
    MissingProviderConfig { model: String, provider: String },
    #[error(transparent)]
    Contract(#[from] TranscriptionContractError),
    #[error(transparent)]
    Config(#[from] ConfigFailure),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Subtitle(#[from] SubtitleTimingError),
    #[error("the request URL {url} is not a URL")]
    Url {
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
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the multipart body could not be encoded")]
    Multipart(#[source] TransportError),
    #[error("the {name} form field is not a primitive value")]
    FormField { name: String },
    #[error("audio transcription data of raw bytes cannot be combined with files")]
    BytesWithFiles,
}

// An httpx.HTTPStatusError carries the masked response: its text and its headers less content-encoding, with content-length recomputed
#[must_use]
pub(super) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = body_text(&reply.body);
    let mut headers = header_map(&reply.headers);
    headers.remove("content-encoding");
    headers.remove("content-length");
    // httpx re-encodes the decoded body into the masked response, so a non-empty one carries a new content-length
    if !reply.body.is_empty() {
        headers.insert(
            String::from("content-length"),
            Json::Text(reply.body.len().to_string()),
        );
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

// httpx turns a TimeoutException into litellm.Timeout before the handler's catch, and any other failure keeps its own text
#[must_use]
pub(super) fn send_failure_facts(
    error: &TransportError,
    mode: ClientKind,
    timeout: Option<Duration>,
    elapsed: Duration,
) -> FailureFacts {
    if caught_timeout(error) {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(mode, timeout, elapsed)),
            headers: Some(HeaderMap::new()),
            response: None,
            display: String::new(),
        };
    }
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    }
}

// Both posts catch httpx.TimeoutException, and a hand-built request reports its timeout as HttpError::Timeout
fn caught_timeout(error: &TransportError) -> bool {
    matches!(
        error,
        TransportError::ConnectTimeout(_)
            | TransportError::Deadline(_)
            | TransportError::Rejected(HttpError::Timeout)
    )
}

// The sync post passes the timeout it was handed; the async post renders the time the failed request took
fn timeout_message(mode: ClientKind, timeout: Option<Duration>, elapsed: Duration) -> String {
    match mode {
        ClientKind::Sync => format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            format_timeout(timeout)
        ),
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
            format_timeout(timeout),
            elapsed.as_secs(),
            elapsed.subsec_millis(),
        ),
    }
}

fn format_timeout(timeout: Option<Duration>) -> String {
    match timeout {
        None => String::from("None"),
        Some(value) if value.subsec_nanos() == 0 => value.as_secs().to_string(),
        Some(value) => value.as_secs_f64().to_string(),
    }
}

fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}
