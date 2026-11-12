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
    EmbeddingRequest as DispatchEmbeddingRequest, EmbeddingResult, EmbeddingRoute, route_error,
};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::client_mock;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
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
        Err(error) => {
            return render(
                route_error(&error, &state.executor.redactor),
                &call_id,
                None,
            );
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone())
        .unwrap_or_else(|| String::from("None"));
    let Ok(deployment) = pick_deployment(&state.executor, &model) else {
        return render(
            route_error(
                &EmbeddingError::UnknownModel { model },
                &state.executor.redactor,
            ),
            &call_id,
            None,
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
        DispatchError::Embedding { source } => render(source.route(redactor), call_id, deployment),
        DispatchError::MissingDeployment { model_id } => render(
            route_error(&EmbeddingError::UnknownModel { model: model_id }, redactor),
            call_id,
            deployment,
        ),
        DispatchError::DeadlineExpired { .. } => render(
            route_error(
                &EmbeddingError::Http {
                    source: HttpError::Timeout,
                },
                redactor,
            ),
            call_id,
            deployment,
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

fn render(route: EmbeddingRoute, call_id: &str, deployment: Option<&ChatDeployment>) -> Response {
    let status_code =
        StatusCode::from_u16(route.failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorBody {
        error: ErrorObject {
            message: route.message,
            error_type: route.failure.error_type,
            param: route.failure.param,
            code: route.failure.status.to_string(),
        },
    };
    (
        status_code,
        failure_headers(None, route.upstream_headers.as_ref(), call_id, deployment),
        Json(body),
    )
        .into_response()
}

fn internal_message(
    message: &str,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    redactor: &Redactor,
) -> Response {
    render(
        EmbeddingRoute::internal(redactor.redact_internal_details(message)),
        call_id,
        deployment,
    )
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
