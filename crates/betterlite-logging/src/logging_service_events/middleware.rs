use std::sync::Arc;

use axum::body::{Body, HttpBody as _};
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::Next;
use axum::response::Response;

use crate::logging_service_events::pipeline::ServicePipeline;
use crate::logging_service_events::stream::{ServiceEventStream, failure_for};
use crate::logging_service_events::types::ServiceKind;

// LiteLLM's call id header, read from the client and stamped back so the handler and the events name one call
const CALL_ID_HEADER: &str = "x-litellm-call-id";

// The pipeline records both events; the server decides how a request's call id is derived.
#[derive(Clone)]
pub struct ServiceEventState {
    pub pipeline: Arc<ServicePipeline>,
    pub call_id: fn(Option<&str>) -> String,
}

// Every chat, text-completion and embeddings call enters the service event track here, one pre-call and one terminal event; a sized body is complete at the handler's return, so it settles there rather than in the stream wrapper.
pub async fn service_events(
    State(state): State<ServiceEventState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(kind) = ServiceKind::from_path(request.uri().path()) else {
        return next.run(request).await;
    };
    let call_id = (state.call_id)(
        request
            .headers()
            .get(CALL_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
    );
    if let Ok(value) = HeaderValue::from_str(&call_id) {
        request.headers_mut().insert(CALL_ID_HEADER, value);
    }
    let call = state.pipeline.begin(kind, &call_id);
    let response = next.run(request).await;
    let (mut parts, body) = response.into_parts();
    let failure = failure_for(parts.status);
    let Some(length) = body.size_hint().exact() else {
        let streamed = ServiceEventStream::new(body.into_data_stream(), call, failure);
        return Response::from_parts(parts, Body::from_stream(streamed));
    };
    call.finish(failure);
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        parts.headers.entry(CONTENT_LENGTH).or_insert(value);
    }
    Response::from_parts(parts, body)
}
