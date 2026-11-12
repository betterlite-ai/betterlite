use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_context::call_context::{CallContext as DispatchContext, CallInputs};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::CallResult;
use betterlite_core::operation::Operation;
use betterlite_core::operation::embedding::{
    EmbeddingCall, EmbeddingCallInput, EmbeddingFailure,
    EmbeddingRequest as DispatchEmbeddingRequest, EmbeddingResult,
};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::client_mock;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::chat::provider_error::{ErrorBodyKind, ProviderFailure, provider_failure};
use betterlite_openai::embedding::{self, EmbeddingError, EmbeddingRequest, EmbeddingResponse};
use betterlite_redact::secret::Redactor;
use serde::Serialize;
use std::str;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

use crate::routing::simple_shuffle::pick_deployment;
use crate::server::chat_setup::provider_call::CallError;
use crate::server::chat_setup::provider_call::embedding::provider_embedding;

use super::chat::headers::{failure_headers, success_headers};

mod tests;

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
}

#[derive(serde::Deserialize)]
struct ProviderErrorBody {
    error: ProviderErrorFields,
}

#[derive(serde::Deserialize)]
struct ProviderErrorFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: Option<String>,
}

pub(crate) fn embedding_routes(executor: Arc<ChatExecutor>) -> Router {
    let calls: Arc<dyn EmbeddingCall> = Arc::new(EmbeddingCalls {
        executor: Arc::clone(&executor),
    });
    let log = Arc::new(TracingCallLog);
    let dispatch = Arc::new(Dispatch::new(
        executor.deployments.clone(),
        Arc::new(NoRules),
        log,
    ));
    let mut router = Router::new();
    router = router.route("/embeddings", post(embeddings));
    router = router.route("/v1/embeddings", post(embeddings));
    router = router.route("/engines/{model}/embeddings", post(embeddings));
    router = router.route("/openai/deployments/{model}/embeddings", post(embeddings));
    router.with_state(EmbeddingState {
        executor,
        calls,
        dispatch,
    })
}

#[derive(Clone)]
struct EmbeddingState {
    executor: Arc<ChatExecutor>,
    calls: Arc<dyn EmbeddingCall>,
    dispatch: Arc<Dispatch>,
}

// the existing embedding implementation, reached through the dispatch pipeline's port
struct EmbeddingCalls {
    executor: Arc<ChatExecutor>,
}

fn embedding_port_failure(error: CallError<EmbeddingError>) -> EmbeddingFailure {
    match error {
        CallError::Route(route) => EmbeddingFailure::Route(route),
        CallError::Operation { source } => EmbeddingFailure::Internal {
            message: source.to_string(),
        },
        CallError::Mismatched => EmbeddingFailure::Mismatched,
    }
}

#[async_trait]
impl EmbeddingCall for EmbeddingCalls {
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure> {
        let (response, upstream_headers, upstream_time) =
            embed(&self.executor, &call.deployment, call.request)
                .await
                .map_err(embedding_port_failure)?;
        Ok(EmbeddingResult {
            response,
            upstream_headers,
            upstream_time,
        })
    }
}

fn routed_call_id(headers: &HeaderMap) -> String {
    headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned)
}

async fn embeddings(
    State(state): State<EmbeddingState>,
    path_model: Option<Path<String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = routed_call_id(&headers);
    let request = match EmbeddingRequest::parse(&body) {
        Ok(request) => request,
        Err(error) => return failure(&error, &call_id, None, &state.executor.redactor),
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone())
        .unwrap_or_else(|| String::from("None"));
    let Ok(deployment) = pick_deployment(&state.executor, &model) else {
        return failure(
            &EmbeddingError::UnknownModel { model },
            &call_id,
            None,
            &state.executor.redactor,
        );
    };
    match run_embedding(&state, request, deployment, &call_id).await {
        Ok(Some(result)) => render_embedding(result, deployment, &call_id, started),
        Ok(None) => internal_message(
            "the embedding call could not be dispatched",
            &call_id,
            Some(deployment),
            &state.executor.redactor,
        ),
        Err(error) => dispatch_failure(error, &call_id, Some(deployment), &state.executor.redactor),
    }
}

fn render_embedding(
    result: EmbeddingResult,
    deployment: &ChatDeployment,
    call_id: &str,
    started: Instant,
) -> Response {
    let response_headers = success_headers(
        &result.upstream_headers,
        deployment,
        call_id,
        result.upstream_time,
        started.elapsed(),
    );
    (response_headers, Json(result.response)).into_response()
}

async fn run_embedding(
    state: &EmbeddingState,
    request: EmbeddingRequest,
    deployment: &ChatDeployment,
    call_id: &str,
) -> Result<Option<EmbeddingResult>, DispatchError> {
    let operation = Operation::Embedding(DispatchEmbeddingRequest {
        call_id: call_id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request),
        calls: Arc::clone(&state.calls),
    });
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::Embedding(result)) => Ok(Some(result)),
        Ok(_) => Ok(None),
        Err(error) => Err(error),
    }
}

fn dispatch_failure(
    error: DispatchError,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    redactor: &Redactor,
) -> Response {
    match error {
        DispatchError::Embedding { source } => {
            embedding_failure_response(source, call_id, deployment, redactor)
        }
        DispatchError::MissingDeployment { model_id } => failure(
            &EmbeddingError::UnknownModel { model: model_id },
            call_id,
            deployment,
            redactor,
        ),
        DispatchError::DeadlineExpired { .. } => failure(
            &EmbeddingError::Http {
                source: HttpError::Timeout,
            },
            call_id,
            deployment,
            redactor,
        ),
        DispatchError::Rules { source } => {
            internal_message(&source.to_string(), call_id, deployment, redactor)
        }
        DispatchError::Chat { source } => {
            internal_message(&source.to_string(), call_id, deployment, redactor)
        }
        DispatchError::Rejected { message, .. } => {
            internal_message(&message, call_id, deployment, redactor)
        }
        DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. } => internal_message(
            "the embedding call could not be dispatched",
            call_id,
            deployment,
            redactor,
        ),
    }
}

fn embedding_failure_response(
    source: EmbeddingFailure,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    redactor: &Redactor,
) -> Response {
    match source {
        EmbeddingFailure::Route(route) => failure(&route, call_id, deployment, redactor),
        EmbeddingFailure::Internal { message } => {
            internal_message(&message, call_id, deployment, redactor)
        }
        EmbeddingFailure::Mismatched => internal_message(
            "the provider answered a different operation",
            call_id,
            deployment,
            redactor,
        ),
    }
}

fn internal_message(
    message: &str,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    redactor: &Redactor,
) -> Response {
    let body = ErrorBody {
        error: ErrorObject {
            message: redactor.redact_internal_details(message),
            error_type: String::from("internal_server_error"),
            param: None,
            code: StatusCode::INTERNAL_SERVER_ERROR.as_u16().to_string(),
        },
    };
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        failure_headers(None, None, call_id, deployment),
        Json(body),
    )
        .into_response()
}

async fn embed(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<(EmbeddingResponse, HeaderMap, Duration), CallError<EmbeddingError>> {
    if embedding_mocks_resolve(executor, deployment, request)? {
        let outcome = embedding::send(executor, deployment, request).await?;
        return Ok((
            outcome.response,
            outcome.upstream.headers,
            outcome.upstream_time,
        ));
    }
    provider_embedding(executor, deployment, request).await
}

fn embedding_mocks_resolve(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<bool, EmbeddingError> {
    let (client_response, client_tool_calls) =
        client_mock(&request.passthrough, executor.mock.permission)
            .map_err(|source| EmbeddingError::Body { source })?;
    let resolved = resolve_mock(
        executor.mock.responses.get(&deployment.model_id),
        client_response.as_ref(),
        client_tool_calls.as_deref(),
        None,
    );
    Ok(resolved.and_then(|plan| plan.response).is_some())
}

fn failure(
    error: &EmbeddingError,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    redactor: &Redactor,
) -> Response {
    let failure = embedding_failure(error, redactor);
    let response_headers = failure_headers_for_embedding(error, call_id, deployment);
    respond(failure, route_message(error), response_headers)
}

fn route_message(error: &EmbeddingError) -> String {
    match error {
        EmbeddingError::Provider { response } => {
            provider_message(response.status, &response.body, error)
        }
        EmbeddingError::Body { .. }
        | EmbeddingError::Unsupported { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Http { .. }
        | EmbeddingError::UnknownModel { .. }
        | EmbeddingError::UnsupportedProvider { .. }
        | EmbeddingError::MissingApiKey
        | EmbeddingError::Endpoint { .. }
        | EmbeddingError::Mock { .. } => error.to_string(),
    }
}

fn respond(failure: ProviderFailure, message: String, headers: HeaderMap) -> Response {
    let status_code =
        StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: failure.error_type,
            param: failure.param,
            code: failure.status.to_string(),
        },
    };
    (status_code, headers, Json(body)).into_response()
}

fn embedding_failure(error: &EmbeddingError, redactor: &Redactor) -> ProviderFailure {
    match error {
        EmbeddingError::Provider { response } => {
            provider_failure(response, redactor, ErrorBodyKind::MessageOnly)
        }
        EmbeddingError::UnknownModel { .. }
        | EmbeddingError::Body { .. }
        | EmbeddingError::Unsupported { .. } => ProviderFailure {
            status: 400,
            error_type: String::from("invalid_request_error"),
            param: None,
        },
        EmbeddingError::MissingApiKey => ProviderFailure {
            status: 401,
            error_type: String::from("authentication_error"),
            param: None,
        },
        EmbeddingError::UnsupportedProvider { .. } => ProviderFailure {
            status: 501,
            error_type: String::from("internal_server_error"),
            param: None,
        },
        EmbeddingError::Http { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Endpoint { .. }
        | EmbeddingError::Mock { .. } => ProviderFailure {
            status: 500,
            error_type: String::from("internal_server_error"),
            param: None,
        },
    }
}

fn failure_headers_for_embedding(
    error: &EmbeddingError,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
) -> HeaderMap {
    let upstream = match error {
        EmbeddingError::Provider { response } => Some(&response.headers),
        EmbeddingError::Body { .. }
        | EmbeddingError::Unsupported { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Http { .. }
        | EmbeddingError::UnknownModel { .. }
        | EmbeddingError::UnsupportedProvider { .. }
        | EmbeddingError::MissingApiKey
        | EmbeddingError::Endpoint { .. }
        | EmbeddingError::Mock { .. } => None,
    };
    failure_headers(None, upstream, call_id, deployment)
}

fn provider_message(status: u16, body: &[u8], fallback: &EmbeddingError) -> String {
    if let Ok(parsed) = serde_json::from_slice::<ProviderErrorBody>(body) {
        let error = parsed.error;
        return format!(
            "Error code: {status} - {{'error': {{'message': '{}', 'type': '{}', 'param': {}, 'code': {}}}}}",
            error.message,
            error.error_type,
            error
                .param
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
            error
                .code
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
        );
    }
    fallback.to_string()
}
