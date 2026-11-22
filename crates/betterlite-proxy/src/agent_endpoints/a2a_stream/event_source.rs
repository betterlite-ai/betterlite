use std::time::Duration;

use async_stream::stream;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::normalize_a2a_jsonrpc_response;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::Method;
use http::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use tokio::time::timeout;
use url::Url;

use super::error::StreamError;
use super::framing::{INTERNAL_ERROR, convert_event, error_event};
use super::lines::{LineSplitter, event_from_line};
use super::ports::{JsonStream, StreamEventConverter};

// LiteLLM's client has no total deadline, but an OutboundRequest needs one and reqwest applies it to the whole body
pub const STREAM_DEADLINE: Duration = Duration::from_hours(24);

// timeout=None makes LiteLLM's client fall back to httpx.Timeout(600), which ends a body read that waits longer
pub const READ_TIMEOUT: Duration = Duration::from_mins(10);

pub struct EventSourceRequest {
    pub agent_url: String,
    pub body: AgentJson,
    pub request_id: Option<AgentJson>,
    pub extra_headers: Vec<(String, String)>,
}

// the status is checked before any line is read, so an error answer is one event even when its body is SSE
#[must_use]
pub fn a2a_sse_event_source<C>(
    client: &ProxiedClient,
    converter: &C,
    request: EventSourceRequest,
) -> JsonStream
where
    C: StreamEventConverter,
{
    let agent = client.clone();
    let event_converter = converter.clone();
    stream! {
        let response = match open(&agent, &request).await {
            Ok(response) => response,
            Err(error) => {
                yield Err(error);
                return;
            }
        };
        let request_id = request.request_id.as_ref();
        if !response.response.status().is_success() {
            yield error_reply(response, agent.limits.body_bytes, request_id).await;
            return;
        }
        let mut events = data_events(response.body_stream(), agent.limits.body_bytes);
        while let Some(event) = events.next().await {
            yield event.map(|parsed| convert_event(&event_converter, parsed, request_id));
        }
    }
    .boxed()
}

async fn open(
    client: &ProxiedClient,
    request: &EventSourceRequest,
) -> Result<StreamResponse, StreamError> {
    let url = Url::parse(&request.agent_url).map_err(StreamError::AgentUrl)?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    // header names compare without case, so an extra "accept" replaces the default Accept
    for (name, value) in &request.extra_headers {
        let header =
            HeaderName::try_from(name.as_str()).map_err(|source| StreamError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::try_from(value.as_str()).map_err(|source| StreamError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        headers.insert(header, header_value);
    }
    let outbound = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(request.body.to_string().into_bytes()),
        deadline: STREAM_DEADLINE,
    };
    // LiteLLM's httpx client follows redirects, so this one does too, with each hop checked
    client
        .send_following(outbound)
        .await
        .map_err(StreamError::Send)
}

// LiteLLM falls back to the generic event when the body is not an object with "error" or does not normalize
async fn error_reply(
    response: StreamResponse,
    limit: usize,
    request_id: Option<&AgentJson>,
) -> Result<AgentJson, StreamError> {
    let reason = response
        .response
        .status()
        .canonical_reason()
        .unwrap_or_default();
    let answer = response
        .buffered(limit)
        .await
        .map_err(StreamError::ErrorBody)?;
    let upstream_error = serde_json::from_slice::<AgentJson>(&answer.body)
        .ok()
        .filter(|parsed| parsed.get("error").is_some())
        .and_then(|parsed| normalize_a2a_jsonrpc_response(&parsed, request_id).ok());
    Ok(upstream_error.unwrap_or_else(|| {
        error_event(
            request_id.unwrap_or(&AgentJson::Null),
            INTERNAL_ERROR,
            reason,
        )
    }))
}

// a line that never ends would grow without bound, so one longer than the client's body limit ends the stream
fn data_events(
    mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    limit: usize,
) -> JsonStream {
    stream! {
        let mut splitter = LineSplitter::default();
        loop {
            let bytes = match timeout(READ_TIMEOUT, body.next()).await {
                Ok(Some(Ok(bytes))) => bytes,
                Ok(Some(Err(source))) => {
                    yield Err(StreamError::Body(source));
                    return;
                }
                Ok(None) => break,
                Err(_idle) => {
                    let seconds = READ_TIMEOUT.as_secs();
                    yield Err(StreamError::ReadTimeout { seconds });
                    return;
                }
            };
            let lines = splitter.push(&bytes);
            for event in lines.iter().map(String::as_str).filter_map(event_from_line) {
                yield Ok(event);
            }
            if splitter.pending.len() > limit {
                yield Err(StreamError::LineTooLong { limit });
                return;
            }
        }
        if let Some(event) = splitter.finish().as_deref().and_then(event_from_line) {
            yield Ok(event);
        }
    }
    .boxed()
}

mod tests;
