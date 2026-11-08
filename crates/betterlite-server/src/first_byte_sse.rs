use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use tokio::time::timeout;

use crate::keepalive_interval::{PingSetting, coerce_keepalive_interval};
use crate::sse_error_frames::{SseFailure, sse_error_frames, sse_error_payload};

const SSE_PING: &[u8] = b": ping\n\n";
const SSE_DONE: &[u8] = b"data: [DONE]\n\n";

pub type ByteStream = BoxStream<'static, Result<Bytes, HttpError>>;

pub struct StreamedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: ByteStream,
}

pub enum ProducedResponse {
    Streaming(StreamedResponse),
    Plain(Response),
}

pub enum HookRaised {
    Replacement(SseFailure),
    Broken(String),
}

#[async_trait]
pub trait LateFailureHook: Send + Sync {
    async fn late_failure(&self, failure: &SseFailure) -> Result<Option<SseFailure>, HookRaised>;
}

pub async fn open_sse_before_first_byte<F>(
    produce_response: F,
    ping_interval_seconds: Option<PingSetting>,
    media_type: &str,
    on_late_failure: Option<Arc<dyn LateFailureHook>>,
) -> Result<Response, SseFailure>
where
    F: Future<Output = Result<ProducedResponse, SseFailure>> + Send + 'static,
{
    let Some(interval) = coerce_keepalive_interval(ping_interval_seconds.as_ref()) else {
        return produce_response.await.map(produced_response);
    };
    let mut produce = Box::pin(produce_response);
    if let Ok(result) = timeout(interval, &mut produce).await {
        return result.map(produced_response);
    }
    tracing::info!(
        "no upstream response after {}s, opening the SSE response early and sending keepalives",
        interval.as_secs_f64()
    );
    let content_type =
        HeaderValue::try_from(format!("{media_type}; charset=utf-8")).map_err(|source| {
            SseFailure {
                status_code: Some(500),
                code: None,
                error_type: None,
                param: None,
                detail: Some(Box::new(JsonValue::String(format!(
                    "the SSE media type is not a valid header value: {source}"
                )))),
                provider_specific_fields: None,
            }
        })?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, content_type);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    let mut response = Response::new(Body::from_stream(relay_stream(
        produce,
        interval,
        on_late_failure,
    )));
    *response.status_mut() = StatusCode::OK;
    *response.headers_mut() = headers;
    Ok(response)
}

fn produced_response(produced: ProducedResponse) -> Response {
    match produced {
        ProducedResponse::Streaming(streamed) => {
            let mut response = Response::new(Body::from_stream(streamed.body));
            *response.status_mut() = streamed.status;
            *response.headers_mut() = streamed.headers;
            response
        }
        ProducedResponse::Plain(response) => response,
    }
}

struct LateRelay {
    interval: Duration,
    failure_hook: Option<Arc<dyn LateFailureHook>>,
    pending: VecDeque<Result<Bytes, HttpError>>,
    streaming: Option<ByteStream>,
    started: bool,
    finished: bool,
}

struct RelayState<F> {
    produce: Pin<Box<F>>,
    relay: LateRelay,
}

fn relay_stream<F>(
    produce: Pin<Box<F>>,
    interval: Duration,
    failure_hook: Option<Arc<dyn LateFailureHook>>,
) -> impl futures_util::Stream<Item = Result<Bytes, HttpError>> + Send
where
    F: Future<Output = Result<ProducedResponse, SseFailure>> + Send + 'static,
{
    let state = RelayState {
        produce,
        relay: LateRelay {
            interval,
            failure_hook,
            pending: VecDeque::new(),
            streaming: None,
            started: false,
            finished: false,
        },
    };
    stream::unfold(state, next_relay_item::<F>)
}

async fn next_relay_item<F>(
    mut state: RelayState<F>,
) -> Option<(Result<Bytes, HttpError>, RelayState<F>)>
where
    F: Future<Output = Result<ProducedResponse, SseFailure>> + Send,
{
    loop {
        if let Some(item) = state.relay.pending.pop_front() {
            return Some((item, state));
        }
        if state.relay.finished {
            return None;
        }
        if let Some(streaming) = state.relay.streaming.as_mut() {
            let pulled = streaming.next().await;
            if let Some(item) = pulled {
                return Some((item, state));
            }
            state.relay.finished = true;
            return None;
        }
        let wait = if state.relay.started {
            state.relay.interval
        } else {
            Duration::ZERO
        };
        state.relay.started = true;
        if let Ok(result) = timeout(wait, &mut state.produce).await {
            finish_late(&mut state.relay, result).await;
            continue;
        }
        state
            .relay
            .pending
            .push_back(Ok(Bytes::from_static(SSE_PING)));
    }
}

async fn finish_late(relay: &mut LateRelay, result: Result<ProducedResponse, SseFailure>) {
    match result {
        Err(failure) => {
            tracing::error!(
                "request failed after its SSE keepalive had opened the response: {failure:?}"
            );
            push_failure_frames(relay, failure).await;
        }
        Ok(ProducedResponse::Streaming(streamed)) => relay.streaming = Some(streamed.body),
        Ok(ProducedResponse::Plain(response)) => {
            let limit = TransportLimits::DEFAULT.body_bytes;
            match to_bytes(response.into_body(), limit).await {
                Ok(body) => push_plain_frames(relay, &body),
                Err(source) => {
                    let failure = SseFailure {
                        status_code: Some(500),
                        code: None,
                        error_type: None,
                        param: None,
                        detail: Some(Box::new(JsonValue::String(format!(
                            "the late response body could not be read: {source}"
                        )))),
                        provider_specific_fields: None,
                    };
                    push_failure_frames(relay, failure).await;
                }
            }
        }
    }
}

async fn push_failure_frames(relay: &mut LateRelay, failure: SseFailure) {
    let effective = apply_hook(relay.failure_hook.as_deref(), failure).await;
    let (_, payload) = sse_error_payload(&effective);
    let [error_frame, done_frame] = sse_error_frames(&payload);
    relay.pending.push_back(Ok(Bytes::from(error_frame)));
    relay.pending.push_back(Ok(Bytes::from(done_frame)));
    relay.finished = true;
}

fn push_plain_frames(relay: &mut LateRelay, body: &Bytes) {
    let mut frame = Vec::new();
    frame.extend_from_slice(b"data: ");
    if body.is_empty() {
        frame.extend_from_slice(b"{}");
    } else {
        frame.extend_from_slice(body);
    }
    frame.extend_from_slice(b"\n\n");
    relay.pending.push_back(Ok(Bytes::from(frame)));
    relay.pending.push_back(Ok(Bytes::from_static(SSE_DONE)));
    relay.finished = true;
}

async fn apply_hook(hook: Option<&dyn LateFailureHook>, failure: SseFailure) -> SseFailure {
    let Some(callback) = hook else {
        return failure;
    };
    match callback.late_failure(&failure).await {
        Ok(Some(replacement)) | Err(HookRaised::Replacement(replacement)) => replacement,
        Ok(None) => failure,
        Err(HookRaised::Broken(message)) => {
            tracing::error!(
                "post_call_failure_hook raised while reporting a late failure: {message}"
            );
            failure
        }
    }
}

mod tests;
