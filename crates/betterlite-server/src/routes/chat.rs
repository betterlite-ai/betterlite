pub(crate) mod headers;
mod streaming;

use streaming::stream_chat;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_error::normalize::{ErrorFacts, normalize_error};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::provider_error::{ErrorBodyKind, classify};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use headers::{failure_headers, success_headers};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    litellm_call_id: Option<String>,
}

// LiteLLM's router repeats its message here when no deployment matches
#[derive(Serialize)]
struct RouterErrorFields {
    error: String,
}

pub(crate) fn chat_routes_with_call_id(
    executor: Arc<ChatExecutor>,
    include_call_id: bool,
) -> Router {
    let router = Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        );
    router.with_state(ChatRouteState {
        executor,
        include_call_id,
    })
}

#[derive(Clone)]
struct ChatRouteState {
    executor: Arc<ChatExecutor>,
    include_call_id: bool,
}

#[derive(Clone, Copy)]
pub(super) struct CallContext<'id> {
    pub(super) id: &'id str,
    pub(super) include_in_body: bool,
}

async fn chat_completion(
    State(state): State<ChatRouteState>,
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
    let call = CallContext {
        id: &call_id,
        include_in_body: state.include_call_id,
    };
    let request = match ChatRequest::parse(&body) {
        Ok(request) => request,
        Err(source) => return failure(&ChatError::from(source), call, None),
    };
    let deployment = match picked_deployment(&state, &request, path_model) {
        Ok(deployment) => deployment,
        Err(error) => return failure(&error, call, None),
    };
    execute_chat(&state, &request, deployment, call, started).await
}

fn picked_deployment<'executor>(
    state: &'executor ChatRouteState,
    request: &ChatRequest,
    path_model: Option<Path<String>>,
) -> Result<&'executor ChatDeployment, ChatError> {
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    state.executor.pick(model.as_deref().unwrap_or("None"))
}

async fn execute_chat(
    state: &ChatRouteState,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    call: CallContext<'_>,
    started: Instant,
) -> Response {
    if request.stream {
        return stream_chat(&state.executor, deployment, request, call, started).await;
    }
    match state.executor.send(deployment, request).await {
        Ok(outcome) => {
            let headers = success_headers(
                &outcome.upstream.headers,
                deployment,
                call.id,
                outcome.upstream_time,
                started.elapsed(),
            );
            (headers, Json(outcome.response)).into_response()
        }
        Err(error) => failure(&error, call, Some(deployment)),
    }
}

fn failure(
    error: &ChatError,
    call: CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    let failure = match error {
        ChatError::Provider { response } => {
            classify(response.status, &response.body, ErrorBodyKind::Structured)
        }
        ChatError::UnknownModel { .. }
        | ChatError::Request { .. }
        | ChatError::Streaming
        | ChatError::UnsupportedProvider { .. }
        | ChatError::MissingApiKey
        | ChatError::Timeout
        | ChatError::Encode { .. }
        | ChatError::Decode { .. }
        | ChatError::TooLarge
        | ChatError::Http { .. } => error.failure(),
    };
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let message = error.to_string();
    log_normalized_error(&message, status.as_u16());
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
            litellm_call_id: call.include_in_body.then(|| call.id.to_owned()),
        },
    };
    let headers = failure_headers(Some(error), None, call.id, deployment);
    (status, headers, Json(body)).into_response()
}

fn log_normalized_error(message: &str, status_code: u16) {
    let facts = ErrorFacts {
        proxy_type: None,
        message: message.to_owned(),
        class: None,
        status_code: Some(status_code),
        rate_limit_type: None,
    };
    let normalized_error = normalize_error(&facts).unwrap_or("UNCLASSIFIED");
    tracing::warn!(normalized_error, "chat request failed");
}

mod tests;
