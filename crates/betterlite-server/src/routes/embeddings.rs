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
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::client_mock;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::chat::provider_error::ProviderFailure;
use betterlite_openai::embedding::{self, EmbeddingError, EmbeddingRequest, EmbeddingResponse};
use betterlite_redact::secret::Redactor;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

use crate::pre_call::rewrites::resolve_call_id;
use crate::route_read_through::{ModelReadThrough, route_with_read_through};
use crate::routing::simple_shuffle::pick_deployment;
use crate::server::RouterErrorFields;
use crate::server::chat_setup::provider_call::CallError;
use crate::server::chat_setup::provider_call::embedding::provider_embedding;

use super::chat::headers::{failure_headers, success_headers};

mod tests;
mod vertex;

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

pub(crate) fn embedding_routes(
    executor: Arc<ChatExecutor>,
    vertex: Option<Arc<dyn EmbeddingCall>>,
    read_through: Option<Arc<dyn ModelReadThrough>>,
) -> Router {
    let calls = vertex::calls(Arc::clone(&executor), vertex);
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
        read_through,
    })
}

#[derive(Clone)]
struct EmbeddingState {
    executor: Arc<ChatExecutor>,
    calls: Arc<dyn EmbeddingCall>,
    dispatch: Arc<Dispatch>,
    read_through: Option<Arc<dyn ModelReadThrough>>,
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

async fn embeddings(
    State(state): State<EmbeddingState>,
    path_model: Option<Path<String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = resolve_call_id(
        headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
    );
    let request = match EmbeddingRequest::parse(&body) {
        Ok(request) => request,
        Err(error) => {
            return render(
                route_error(&error, &state.executor.redactor),
                &call_id,
                None,
                None,
            );
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    let model_text = model.as_deref().unwrap_or("None");
    let route = route_endpoint("aembedding").unwrap_or("aembedding");
    let attempt = || async {
        pick_deployment(&state.executor, model_text)
            .map_err(|_error| ProxyModelNotFoundError::new(route, model_text, true))
    };
    let deployment =
        match route_with_read_through(model.as_deref(), state.read_through.as_deref(), attempt)
            .await
        {
            Ok(deployment) => deployment,
            Err(error) => {
                return not_found_response(&error, &call_id, &state.executor.redactor);
            }
        };
    execute(&state, request, deployment, &call_id, started).await
}

async fn execute(
    state: &EmbeddingState,
    request: EmbeddingRequest,
    deployment: &ChatDeployment,
    call_id: &str,
    started: Instant,
) -> Response {
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
    let redactor = &state.executor.redactor;
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::Embedding(result)) => {
            let response_headers = success_headers(
                &result.upstream_headers,
                deployment,
                call_id,
                result.upstream_time,
                started.elapsed(),
            );
            (response_headers, Json(result.response)).into_response()
        }
        Ok(_) => {
            let unbuilt = DispatchError::NotBuilt { entry: "embedding" };
            dispatch_failure(unbuilt, call_id, Some(deployment), redactor)
        }
        Err(error) => dispatch_failure(error, call_id, Some(deployment), redactor),
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
            render(source.route(redactor), call_id, deployment, None)
        }
        DispatchError::MissingDeployment { model_id } => render(
            route_error(&EmbeddingError::UnknownModel { model: model_id }, redactor),
            call_id,
            deployment,
            None,
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
            None,
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

fn render(
    route: EmbeddingRoute,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
    provider_specific_fields: Option<String>,
) -> Response {
    let status_code =
        StatusCode::from_u16(route.failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorBody {
        error: ErrorObject {
            message: route.message,
            error_type: route.failure.error_type,
            param: route.failure.param,
            code: route.failure.status.to_string(),
            provider_specific_fields: provider_specific_fields
                .map(|error| RouterErrorFields { error }),
        },
    };
    (
        status_code,
        failure_headers(None, route.upstream_headers.as_ref(), call_id, deployment),
        Json(body),
    )
        .into_response()
}

fn not_found_response(
    error: &ProxyModelNotFoundError,
    call_id: &str,
    redactor: &Redactor,
) -> Response {
    let message = redactor.redact_internal_details(&error.detail.error);
    let route = EmbeddingRoute {
        failure: ProviderFailure {
            status: ProxyModelNotFoundError::STATUS.as_u16(),
            error_type: String::from("invalid_request_error"),
            param: None,
        },
        message: message.clone(),
        upstream_headers: None,
    };
    render(route, call_id, None, Some(message))
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
        None,
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
