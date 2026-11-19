use axum::body::Bytes;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::Response;
use betterlite_core::operation::chat::ChatFailure;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::chat::{ChatRequest, RequestError};
use std::sync::Arc;

use crate::route_read_through::route_with_read_through;
use crate::routing::simple_shuffle::pick_deployment;
use crate::server::failure;
use crate::server::render::{FailureShape, plain_failure};
use crate::state::ServerState;

use super::{CallContext, ChatRouteState};

pub(super) fn parse_chat_request(
    body: &Bytes,
    call: &mut CallContext<'_>,
) -> Result<ChatRequest, ChatError> {
    let request = match ChatRequest::parse(body) {
        Ok(request) => request,
        Err(source) => {
            if let RequestError::Body { source: json } = &source {
                // LiteLLM's _read_request_body treats an empty body as {} instead of raising.
                call.include_in_body &= body.is_empty() || !(json.is_syntax() || json.is_eof());
            }
            return Err(ChatError::from(source));
        }
    };
    Ok(request)
}

pub(super) async fn routed_deployment(
    state: &ChatRouteState,
    model: Option<&str>,
    server: Option<Extension<Arc<ServerState>>>,
) -> Result<ChatDeployment, ProxyModelNotFoundError> {
    let model_text = model.map(str::to_owned);
    let read_through =
        server.and_then(|Extension(server)| server.model_read_through.read_through.clone());
    let executor = Arc::clone(&state.executor);
    // the AsyncFn bound cannot prove Send for a captured reference, so the attempt owns its reads
    let attempt = move || {
        let shared_executor = Arc::clone(&executor);
        let shared_model = model_text.clone();
        async move {
            let text = shared_model.as_deref().unwrap_or("None");
            let route = route_endpoint("acompletion").unwrap_or("acompletion");
            let deployment = pick_deployment(&shared_executor, text)
                .map_err(|_error| ProxyModelNotFoundError::new(route, text, true))?;
            Ok(deployment.clone())
        }
    };
    route_with_read_through(model, read_through.as_deref(), attempt).await
}

pub(super) fn not_found_failure(
    call: &CallContext<'_>,
    error: &ProxyModelNotFoundError,
) -> Response {
    let message = call
        .errors
        .redactor
        .redact_internal_details(&error.detail.error);
    tracing::warn!(error = %message, "chat request failed before dispatch");
    plain_failure(
        call,
        None,
        FailureShape {
            status: ProxyModelNotFoundError::STATUS,
            error_type: "invalid_request_error",
            param: None,
            message,
            repeat_message: true,
        },
    )
}

pub(super) fn chat_failure(
    chat_error: &ChatFailure,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    if let ChatFailure::Route(route) = chat_error {
        return failure(route, call, deployment);
    }
    if let ChatFailure::InvalidControl { source } = chat_error {
        let param = Some(source.param().to_owned());
        return plain_failure(
            call,
            deployment,
            FailureShape {
                status: StatusCode::BAD_REQUEST,
                error_type: "invalid_request_error",
                param,
                message: source.to_string(),
                repeat_message: false,
            },
        );
    }
    internal_failure(&chat_error.to_string(), call, deployment)
}

pub(super) fn internal_failure(
    message: &str,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    let redacted = call.errors.redactor.redact_internal_details(message);
    plain_failure(
        call,
        deployment,
        FailureShape {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_type: "internal_server_error",
            param: None,
            message: redacted,
            repeat_message: false,
        },
    )
}
