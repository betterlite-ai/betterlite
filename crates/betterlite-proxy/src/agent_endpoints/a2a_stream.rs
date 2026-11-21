pub mod error;
pub mod event_source;
pub mod framing;
pub mod keepalive;
pub mod lines;
pub mod ports;

use std::collections::BTreeMap;

use async_stream::stream;
use betterlite_a2a::send_params::build_message_send_params;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::ProxiedClient;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};

use self::error::{StreamAbort, StreamFailure};
use self::event_source::{EventSourceRequest, a2a_sse_event_source};
use self::framing::{
    INTERNAL_ERROR, INVALID_PARAMS, convert_event, error_event, to_jsonrpc_object,
};
use self::keepalive::{coerce_keepalive_interval, with_pings};
use self::ports::{
    HookedStream, JsonStream, StreamEventConverter, StreamHooks, StreamOpen, UpstreamAgentStream,
};

pub const MEDIA_TYPE: &str = "text/event-stream";

// a proxy that buffers the whole stream would swallow the pings, so these go out with them
const KEEPALIVE_HEADERS: [(&str, &str); 2] =
    [("Cache-Control", "no-cache"), ("X-Accel-Buffering", "no")];

pub type FrameStream = BoxStream<'static, Result<String, StreamAbort>>;

pub struct SseResponse {
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: FrameStream,
}

pub struct MessageStreamRequest {
    pub request_id: AgentJson,
    pub params: AgentJson,
    pub api_base: Option<String>,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub agent_id: Option<String>,
    pub metadata: Option<AgentJson>,
    pub proxy_server_request: Option<AgentJson>,
    pub agent_extra_headers: Option<Vec<(String, String)>>,
}

#[must_use]
pub fn sse_response(frames: FrameStream, ping_interval: Option<f64>) -> SseResponse {
    match coerce_keepalive_interval(ping_interval) {
        Some(interval) => SseResponse {
            headers: KEEPALIVE_HEADERS.to_vec(),
            body: with_pings(frames, interval),
        },
        None => SseResponse {
            headers: Vec::new(),
            body: frames,
        },
    }
}

// the invalid-params answer skips sse_response, so it never carries pings or their headers
#[must_use]
pub fn relay_message_stream<U, C, H>(
    upstream: &U,
    converter: &C,
    hooks: Option<&H>,
    request: MessageStreamRequest,
    ping_interval: Option<f64>,
) -> SseResponse
where
    U: UpstreamAgentStream,
    C: StreamEventConverter,
    H: StreamHooks,
{
    let params = match build_message_send_params(&request.params) {
        Ok(params) => params,
        Err(error) => {
            let message = format!("Invalid params: {error}");
            let event = error_event(&request.request_id, INVALID_PARAMS, &message);
            return SseResponse {
                headers: Vec::new(),
                body: iter([Ok(format!("data: {event}\n\n"))]).boxed(),
            };
        }
    };
    let request_id = request.request_id.clone();
    let events = upstream.stream(StreamOpen {
        request_id: request.request_id,
        params,
        api_base: request.api_base,
        litellm_params: request.litellm_params,
        agent_id: request.agent_id,
        metadata: request.metadata,
        proxy_server_request: request.proxy_server_request,
        agent_extra_headers: request.agent_extra_headers,
    });
    let chunk_converter = converter.clone();
    let chunk_id = request_id.clone();
    let sse_chunk = move |chunk: AgentJson| {
        let event = convert_event(&chunk_converter, to_jsonrpc_object(chunk), Some(&chunk_id));
        format!("data: {event}\n\n")
    };
    let frames = match hooks {
        Some(stream_hooks) => hooked_frames(stream_hooks.serve(events), request_id, sse_chunk),
        None => unhooked_frames(events, request_id, sse_chunk),
    };
    sse_response(frames, ping_interval)
}

#[must_use]
pub fn relay_task_subscription<C, H>(
    client: &ProxiedClient,
    converter: &C,
    hooks: Option<&H>,
    request: EventSourceRequest,
    ping_interval: Option<f64>,
) -> SseResponse
where
    C: StreamEventConverter,
    H: StreamHooks,
{
    let request_id = request.request_id.clone().unwrap_or_default();
    let events = a2a_sse_event_source(client, converter, request);
    let frames = match hooks {
        Some(stream_hooks) => hooked_frames(
            stream_hooks.serve(events),
            request_id,
            |event: AgentJson| format!("data: {event}\n\n"),
        ),
        // without hooks LiteLLM lets the error escape the generator, so the body ends with no error frame
        None => events
            .map(|item| match item {
                Ok(event) => Ok(format!("data: {event}\n\n")),
                Err(error) => Err(StreamAbort {
                    message: error.to_string(),
                }),
            })
            .boxed(),
    };
    sse_response(frames, ping_interval)
}

// the hooks already ran the failure hook and redacted the message, so Failed is sent as it is
fn hooked_frames<F>(mut events: HookedStream, request_id: AgentJson, frame: F) -> FrameStream
where
    F: Fn(AgentJson) -> String + Send + Sync + 'static,
{
    stream! {
        while let Some(item) = events.next().await {
            match item {
                Ok(chunk) => yield Ok(frame(chunk)),
                Err(StreamFailure::Failed { message }) => {
                    let event = error_event(&request_id, INTERNAL_ERROR, &message);
                    yield Ok(format!("data: {event}\n\n"));
                    return;
                }
                Err(StreamFailure::Rejected { detail, .. }) => {
                    yield Err(StreamAbort { message: detail });
                    return;
                }
            }
        }
    }
    .boxed()
}

fn unhooked_frames<F>(mut events: JsonStream, request_id: AgentJson, frame: F) -> FrameStream
where
    F: Fn(AgentJson) -> String + Send + Sync + 'static,
{
    stream! {
        while let Some(item) = events.next().await {
            match item {
                Ok(chunk) => yield Ok(frame(chunk)),
                Err(error) => {
                    tracing::error!("Error streaming A2A response: {error}");
                    let message = format!("Streaming error: {error}");
                    let event = error_event(&request_id, INTERNAL_ERROR, &message);
                    yield Ok(format!("data: {event}\n\n"));
                    return;
                }
            }
        }
    }
    .boxed()
}

mod tests;
