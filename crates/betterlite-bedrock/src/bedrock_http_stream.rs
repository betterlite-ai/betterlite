use std::str;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;
use url::Url;

use crate::agentcore_stream::FIRST_RESPONSE;
use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::transport::{PostRequest, StreamHandler, StreamHead};
use crate::invoke_events::DecoderClock;

pub mod converse;
pub mod error;
pub mod fake;
pub mod invoke;
pub mod stream;

mod tests;

// The client configuration a stream call runs with; LiteLLM's make_call sees the same values through its client and logging object.
pub struct StreamDeps<'deps> {
    pub supplied: Option<Arc<dyn StreamHandler>>,
    pub default: Arc<dyn StreamHandler>,
    pub ids: Arc<dyn IdSource>,
    pub clock: DecoderClock,
    pub redactor: &'deps Redactor,
    pub timeout: Duration,
    // The fresh ModelResponse a fake stream converts carries int(time.time()); the caller supplies it.
    pub now: SystemTime,
}

pub(crate) struct Opening<'call> {
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub stream: bool,
}

// What the POST phase leaves for the sync phase: the opened response and, for a fake stream, the body it converts.
pub struct PendingOpen {
    pub head: StreamHead,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Error)]
pub(crate) enum SendFailure {
    #[error("the api_base is not a URL")]
    Url(#[source] url::ParseError),
    #[error("the Bedrock stream request failed")]
    Transport(#[source] TransportError),
}

pub(crate) async fn send(
    handler: &dyn StreamHandler,
    opening: Opening<'_>,
    timeout: Duration,
) -> Result<StreamHead, SendFailure> {
    let url = Url::parse(opening.api_base).map_err(SendFailure::Url)?;
    handler
        .post(PostRequest {
            url,
            headers: opening.headers,
            body: opening.body,
            stream: opening.stream,
            deadline: timeout,
        })
        .await
        .map_err(SendFailure::Transport)
}

// response.text, and a body that is not UTF-8 is named by its byte count instead of decoding replacement characters.
pub(crate) fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not text", body.len()),
        str::to_owned,
    )
}

// The opened stream is logged before anything reads from it, and the request body stands in for the buffered response.
pub(crate) fn log_open(
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    messages: &Json,
    data: &str,
) {
    post_call(
        state,
        &PostCall {
            original_response: Json::Text(FIRST_RESPONSE.to_owned()),
            input: messages.clone(),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Text(data.to_owned())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    );
}
