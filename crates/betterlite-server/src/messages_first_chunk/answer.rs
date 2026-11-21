use std::sync::Arc;

use async_trait::async_trait;
use axum::Json as AxumJson;
use axum::body::Bytes;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse as _;
use betterlite_anthropic::api::messages_errors::AnthropicErrorSseFrame;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::envelope::error_body_call_id;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::frame_error::{frame_error_code, frame_error_object};
use crate::first_byte_sse::{ByteStream, ProducedResponse, StreamedResponse};
use crate::shared_stream::call::StreamAbort;
use crate::shared_stream::sse::Emitted;
use crate::sse_error_frames::{SseFailure, sse_error_frames, sse_error_payload};

const CALL_ID_HEADER: &str = "x-litellm-call-id";
const CALL_ID_KEY: &str = "litellm_call_id";
const ERROR_KEY: &str = "error";
const SSE_CONTENT_TYPE: &str = "text/event-stream; charset=utf-8";
const BODY_FAILURE_LOG: &str = "the first-chunk error body could not be built";

// A refresher is B081's rebuild of the headers for the deployment that served the stream; its refusal is only logged
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HeaderRefreshError {
    pub message: String,
}

#[async_trait]
pub trait HeaderRefresh: Send + Sync {
    async fn refresh(&self) -> Result<HeaderMap, HeaderRefreshError>;
}

// _resolve_stream_headers: the caller's headers unless a refresher answers; a failure keeps the caller's
pub async fn resolve_headers(
    headers: &HeaderMap,
    refresh: Option<&dyn HeaderRefresh>,
) -> HeaderMap {
    let Some(rebuild) = refresh else {
        return headers.clone();
    };
    match rebuild.refresh().await {
        Ok(updated) => updated,
        Err(error) => {
            tracing::error!("Error refreshing streaming response headers: {error}");
            headers.clone()
        }
    }
}

// _sse_stream_headers: the two headers that stop a reverse proxy from buffering the stream
#[must_use]
pub fn sse_stream_headers(mut headers: HeaderMap) -> HeaderMap {
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    headers
}

// create_response with media type text/event-stream and status 200: only the first item decides the shape
pub async fn first_chunk_answer(
    mut body: BoxStream<'static, Result<Emitted, StreamAbort>>,
    headers: HeaderMap,
    refresh: Option<Arc<dyn HeaderRefresh>>,
    call_id_in_body: bool,
) -> ProducedResponse {
    let Some(opening) = body.next().await else {
        let settled = resolve_headers(&headers, refresh.as_deref()).await;
        return sse_answer(StatusCode::OK, settled, stream::empty().boxed());
    };
    let first = match opening {
        Ok(item) => item,
        Err(abort) => {
            tracing::error!(
                "Error consuming first chunk from generator: {}",
                abort.detail
            );
            let failure = sse_failure_from_detail(Some(abort.status), abort.detail);
            return sse_failure_answer(&failure, &headers, refresh.as_deref()).await;
        }
    };
    let resolved = resolve_headers(&headers, refresh.as_deref()).await;
    match first {
        Emitted::AnthropicError(frame) => {
            anthropic_error_answer(
                frame.as_ref(),
                resolved,
                &headers,
                refresh.as_deref(),
                call_id_in_body,
            )
            .await
        }
        Emitted::Frame(bytes) => frame_answer(bytes, resolved, body, call_id_in_body),
    }
}

// LiteLLM answers an error frame before its first byte as the JSON error the frame was built from
async fn anthropic_error_answer(
    frame: &AnthropicErrorSseFrame,
    resolved: HeaderMap,
    headers: &HeaderMap,
    refresh: Option<&dyn HeaderRefresh>,
    call_id_in_body: bool,
) -> ProducedResponse {
    let identifier = call_id(&resolved, call_id_in_body);
    match frame.json_body(identifier) {
        Ok(answer) => {
            let status = StatusCode::from_u16(frame.status_code)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            ProducedResponse::Plain((status, resolved, AxumJson(answer)).into_response())
        }
        Err(error) => {
            tracing::error!(message = BODY_FAILURE_LOG, error = %error);
            let failure = sse_failure_from_detail(None, error.to_string());
            sse_failure_answer(&failure, headers, refresh).await
        }
    }
}

// A first data chunk with an error code answers JSON; any other chunk opens the stream with it
fn frame_answer(
    bytes: Bytes,
    resolved: HeaderMap,
    body: BoxStream<'static, Result<Emitted, StreamAbort>>,
    call_id_in_body: bool,
) -> ProducedResponse {
    let Some(code) = frame_error_code(&bytes) else {
        return sse_answer(StatusCode::OK, resolved, combined_stream(bytes, body));
    };
    tracing::debug!(
        "Error detected in first stream chunk. Returning JSON error response with status code: {code}"
    );
    let mut error = frame_error_object(&bytes);
    if let Some(identifier) = call_id(&resolved, call_id_in_body) {
        error.insert(String::from(CALL_ID_KEY), Json::Text(identifier.to_owned()));
    }
    let mut envelope = JsonObject::new();
    envelope.insert(String::from(ERROR_KEY), Json::Object(error));
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    ProducedResponse::Plain((status, resolved, AxumJson(Json::Object(envelope))).into_response())
}

async fn sse_failure_answer(
    failure: &SseFailure,
    headers: &HeaderMap,
    refresh: Option<&dyn HeaderRefresh>,
) -> ProducedResponse {
    let (failure_status, payload) = sse_error_payload(failure);
    let frames = sse_error_frames(&payload);
    let settled = resolve_headers(headers, refresh).await;
    let status = StatusCode::from_u16(failure_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    sse_answer(status, settled, frame_bytes(frames))
}

fn call_id(headers: &HeaderMap, call_id_in_body: bool) -> Option<&str> {
    let identifier = headers
        .get(CALL_ID_HEADER)
        .and_then(|value| value.to_str().ok());
    error_body_call_id(call_id_in_body, identifier)
}

fn sse_failure_from_detail(status_code: Option<u16>, detail: String) -> SseFailure {
    SseFailure {
        status_code,
        code: None,
        error_type: None,
        param: None,
        detail: Some(Box::new(JsonValue::String(detail))),
        provider_specific_fields: None,
    }
}

fn sse_answer(status: StatusCode, headers: HeaderMap, body: ByteStream) -> ProducedResponse {
    let mut settled = sse_stream_headers(headers);
    settled.insert(CONTENT_TYPE, HeaderValue::from_static(SSE_CONTENT_TYPE));
    ProducedResponse::Streaming(StreamedResponse {
        status,
        headers: settled,
        body,
    })
}

fn frame_bytes(frames: [String; 2]) -> ByteStream {
    let [error_frame, done_frame] = frames;
    stream::iter([Ok(Bytes::from(error_frame)), Ok(Bytes::from(done_frame))]).boxed()
}

struct Relay {
    pending: Option<Bytes>,
    rest: Option<BoxStream<'static, Result<Emitted, StreamAbort>>>,
}

fn combined_stream(
    first: Bytes,
    rest: BoxStream<'static, Result<Emitted, StreamAbort>>,
) -> ByteStream {
    let state = Relay {
        pending: Some(first),
        rest: Some(rest),
    };
    stream::unfold(state, next_relay_bytes).boxed()
}

async fn next_relay_bytes(mut state: Relay) -> Option<(Result<Bytes, HttpError>, Relay)> {
    if let Some(bytes) = state.pending.take() {
        return Some((Ok(bytes), state));
    }
    let mut rest = state.rest.take()?;
    match rest.next().await {
        Some(Ok(item)) => {
            state.rest = Some(rest);
            Some((Ok(item.into_bytes()), state))
        }
        Some(Err(abort)) => {
            tracing::error!(
                "Error consuming a stream chunk after the first one: {} (status {})",
                abort.detail,
                abort.status
            );
            None
        }
        None => None,
    }
}
