use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::speech_provider_contract::error::SpeechContractError;
use crate::api::speech_provider_contract::{SpeechHeaders, TextToSpeechConfig};

// LiteLLM's BaseLLMException together with the flag only _handle_error sets
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SpeechProviderError {
    pub status_code: u16,
    pub message: String,
    pub headers: SpeechHeaders,
    pub status_code_is_synthesized: bool,
}

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error(transparent)]
    Provider(#[from] SpeechProviderError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    CompleteUrl(#[from] SpeechContractError),
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
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResponseFacts {
    pub(super) text: Option<String>,
    pub(super) headers: Option<SpeechHeaders>,
}

// Everything _handle_error reads off the caught exception before it picks a status, a text and a header map
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailureFacts {
    pub(super) response_status: Option<i64>,
    pub(super) status: Option<u16>,
    pub(super) text: Option<String>,
    pub(super) headers: Option<SpeechHeaders>,
    pub(super) response: Option<ResponseFacts>,
    pub(super) display: String,
}

// The shared error converter: a response status error keeps its response status and text, another exception its own integer status, and anything else becomes a synthesized 500
pub(super) fn handle_error(
    facts: &FailureFacts,
    config: &dyn TextToSpeechConfig,
) -> SpeechProviderError {
    let received = facts
        .response_status
        .map_or(facts.status, |status| u16::try_from(status).ok());
    let exception = config.get_error_class(
        &error_text(facts),
        received.unwrap_or(500),
        &error_headers(facts),
    );
    SpeechProviderError {
        status_code: exception.status_code,
        message: exception.message,
        headers: exception.headers,
        status_code_is_synthesized: received.is_none(),
    }
}

// httpx's HTTPStatusError reports the response body; any other exception only its own text or str()
fn error_text(facts: &FailureFacts) -> String {
    let response_text = facts
        .response
        .as_ref()
        .and_then(|response| response.text.clone());
    match (&facts.response_status, response_text) {
        (Some(_), Some(text)) => text,
        _ => facts.text.clone().unwrap_or_else(|| facts.display.clone()),
    }
}

// The exception's own headers win; otherwise the response's, and an exception carrying neither has none
fn error_headers(facts: &FailureFacts) -> SpeechHeaders {
    facts
        .headers
        .clone()
        .or_else(|| facts.response.as_ref()?.headers.clone())
        .unwrap_or_default()
}
