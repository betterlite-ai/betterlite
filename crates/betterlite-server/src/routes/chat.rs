mod headers;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_http::proxy::StreamResponse;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use serde::Serialize;
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use futures_util::stream::{self, BoxStream};
use futures_util::{Stream as _, StreamExt as _};

use betterlite_openai::stream::SseParser;
use headers::{failure_headers, stream_headers, success_headers};

type UpstreamStream = BoxStream<'static, Result<Vec<u8>, HttpError>>;
type RelayState = (UpstreamStream, SseParser, VecDeque<Bytes>, bool);

// LiteLLM's MAX_LITELLM_CALL_ID_LENGTH
const MAX_CALL_ID_LENGTH: usize = 256;

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorObject,
}

#[derive(Serialize)]
struct ErrorObject {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_specific_fields: Option<RouterErrorFields>,
}

// LiteLLM's router repeats its message here when no deployment matches
#[derive(Serialize)]
struct RouterErrorFields {
    error: String,
}

pub(crate) fn chat_routes(executor: Arc<ChatExecutor>) -> Router {
    let router = Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        );
    router.with_state(executor)
}

async fn chat_completion(
    State(executor): State<Arc<ChatExecutor>>,
    path_model: Option<Path<String>>,
    request_headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = request_headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (1..=MAX_CALL_ID_LENGTH).contains(&value.len()))
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
    let request = match ChatRequest::parse(&body) {
        Ok(request) => request,
        Err(source) => return failure(&ChatError::from(source), &call_id, None),
    };
    // LiteLLM lets the model in the path win over the one in the body
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    let deployment = match executor.pick(model.as_deref().unwrap_or("None")) {
        Ok(deployment) => deployment,
        Err(error) => return failure(&error, &call_id, None),
    };
    if request.stream {
        return stream_chat(&executor, deployment, &request, &call_id).await;
    }
    match executor.send(deployment, &request).await {
        Ok(outcome) => {
            let headers = success_headers(&outcome, deployment, &call_id, started.elapsed());
            (headers, Json(outcome.response)).into_response()
        }
        Err(error) => failure(&error, &call_id, Some(deployment)),
    }
}

async fn stream_chat(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    call_id: &str,
) -> Response {
    match executor.stream(deployment, request).await {
        Ok(upstream) if upstream.response.status().is_success() => {
            let headers = stream_headers(&upstream, call_id);
            let stream_body = Body::from_stream(relay_sse(upstream));
            (headers, stream_body).into_response()
        }
        Ok(upstream) => {
            let error = ChatError::Provider {
                response: Box::new(UpstreamResponse {
                    status: upstream.response.status().as_u16(),
                    headers: upstream.response.headers().clone(),
                    body: Vec::new(),
                }),
            };
            failure(&error, call_id, Some(deployment))
        }
        Err(error) => failure(&error, call_id, Some(deployment)),
    }
}

fn relay_sse(
    upstream_response: StreamResponse,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> {
    let stream = stream::unfold(
        (
            upstream_response.body_stream(),
            SseParser::default(),
            VecDeque::new(),
            false,
        ),
        next_sse,
    );
    if stream.size_hint().0 > 0 {
        return stream;
    }
    stream
}

async fn next_sse(
    (mut upstream, mut parser, mut pending, mut finished): RelayState,
) -> Option<(Result<Bytes, io::Error>, RelayState)> {
    loop {
        if let Some(event) = pending.pop_front() {
            return Some((Ok(event), (upstream, parser, pending, finished)));
        }
        if finished {
            return None;
        }
        match upstream.next().await {
            Some(Ok(chunk)) => append_events(&mut pending, parser.push(&chunk)),
            Some(Err(error)) => {
                return Some((
                    Err(io::Error::other(error)),
                    (upstream, parser, pending, true),
                ));
            }
            None => {
                append_events(&mut pending, parser.finish());
                finished = true;
            }
        }
    }
}

fn append_events(pending: &mut VecDeque<Bytes>, events: Vec<Vec<u8>>) {
    for event in events {
        pending.push_back(format_sse_event(event));
    }
}

fn format_sse_event(event: Vec<u8>) -> Bytes {
    let text = String::from_utf8(event).unwrap_or_else(|_| String::from("[DONE]"));
    Bytes::from(format!("data: {text}\n\n"))
}

fn failure(error: &ChatError, call_id: &str, deployment: Option<&ChatDeployment>) -> Response {
    let failure = error.failure();
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let message = error.to_string();
    let provider_specific_fields =
        matches!(error, ChatError::UnknownModel { .. }).then(|| RouterErrorFields {
            error: message.clone(),
        });
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: failure.error_type,
            param: failure.param,
            code: status.as_u16().to_string(),
            provider_specific_fields,
        },
    };
    let headers = failure_headers(error, call_id, deployment);
    (status, headers, Json(body)).into_response()
}

mod tests;
