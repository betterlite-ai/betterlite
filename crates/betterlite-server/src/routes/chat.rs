pub(crate) mod headers;
mod streaming;

use streaming::stream_chat;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_error::envelope::{EnvelopeError, error_body_call_id};
use betterlite_error::normalize::class::Class;
use betterlite_error::normalize::provider::provider_facts;
use betterlite_error::normalize::{ErrorFacts, Normalizer};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::{ChatRequest, RequestError};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::routing::simple_shuffle::pick_deployment;

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
) -> Result<Router, EnvelopeError> {
    let router = Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        );
    Ok(router.with_state(ChatRouteState {
        executor,
        include_call_id,
        errors: Arc::new(ErrorLayer {
            normalizer: Normalizer::new()?,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
        }),
    }))
}

struct ErrorLayer {
    normalizer: Normalizer,
    redactor: Redactor,
}

#[derive(Clone)]
struct ChatRouteState {
    executor: Arc<ChatExecutor>,
    include_call_id: bool,
    errors: Arc<ErrorLayer>,
}

#[derive(Clone)]
pub(super) struct CallContext<'id> {
    pub(super) id: &'id str,
    pub(super) include_in_body: bool,
    pub(super) started: Instant,
    errors: &'id ErrorLayer,
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
    let mut call = CallContext {
        id: &call_id,
        include_in_body: state.include_call_id,
        started,
        errors: &state.errors,
    };
    let request = match ChatRequest::parse(&body) {
        Ok(request) => request,
        Err(source) => {
            if let RequestError::Body { source: json } = &source {
                // LiteLLM's _read_request_body treats an empty body as {} instead of raising.
                call.include_in_body &= body.is_empty() || !(json.is_syntax() || json.is_eof());
            }
            return failure(&ChatError::from(source), &call, None);
        }
    };
    let deployment = match picked_deployment(&state, &request, path_model) {
        Ok(deployment) => deployment,
        Err(error) => return failure(&error, &call, None),
    };
    execute_chat(&state, &request, deployment, &call).await
}

fn picked_deployment<'executor>(
    state: &'executor ChatRouteState,
    request: &ChatRequest,
    path_model: Option<Path<String>>,
) -> Result<&'executor ChatDeployment, ChatError> {
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    pick_deployment(&state.executor, model.as_deref().unwrap_or("None"))
}

async fn execute_chat(
    state: &ChatRouteState,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    call: &CallContext<'_>,
) -> Response {
    if request.stream {
        return stream_chat(&state.executor, deployment, request, call).await;
    }
    match state.executor.send(deployment, request).await {
        Ok(outcome) => {
            let headers = success_headers(
                &outcome.upstream.headers,
                deployment,
                call.id,
                outcome.upstream_time,
                call.started.elapsed(),
            );
            (headers, Json(outcome.response)).into_response()
        }
        Err(error) => failure(&error, call, Some(deployment)),
    }
}

fn failure(
    error: &ChatError,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    let failure = error.failure();
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let redactor = &call.errors.redactor;
    let message = redactor.redact_internal_details(&error.to_string());
    log_normalized_error(error, &message, status.as_u16(), call.errors);
    let provider_specific_fields =
        matches!(error, ChatError::UnknownModel { .. }).then(|| RouterErrorFields {
            error: message.clone(),
        });
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: redactor.redact_internal_details(&failure.error_type),
            param: failure
                .param
                .map(|param| redactor.redact_internal_details(&param)),
            code: status.as_u16().to_string(),
            provider_specific_fields,
            litellm_call_id: error_body_call_id(call.include_in_body, Some(call.id))
                .map(str::to_owned),
        },
    };
    let headers = failure_headers(Some(error), None, call.id, deployment);
    (status, headers, Json(body)).into_response()
}

fn log_normalized_error(error: &ChatError, message: &str, status: u16, layer: &ErrorLayer) {
    let facts = if let ChatError::Provider { response } = error {
        provider_facts(response.status, &response.body, &layer.redactor)
    } else {
        let class = match error {
            ChatError::Timeout => Class::Timeout,
            ChatError::Http { .. } => Class::APIConnectionError,
            ChatError::UnknownModel { .. }
            | ChatError::Request { .. }
            | ChatError::Streaming
            | ChatError::UnsupportedProvider { .. }
            | ChatError::MissingApiKey
            | ChatError::Provider { .. }
            | ChatError::Encode { .. }
            | ChatError::Decode { .. }
            | ChatError::TooLarge => Class::Other,
        };
        ErrorFacts {
            proxy_type: None,
            message: message.to_owned(),
            class,
            status_code: Some(status),
            rate_limit_type: None,
        }
    };
    let normalized_error = layer.normalizer.normalize(Some(&facts));
    tracing::warn!(normalized_error, "chat request failed");
}

mod tests;
