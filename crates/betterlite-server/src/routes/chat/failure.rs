use axum::http::StatusCode;
use axum::response::Response;
use betterlite_core::operation::chat::ChatFailure;
use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_openai::chat::deployment::ChatDeployment;

use crate::server::failure;
use crate::server::render::{FailureShape, plain_failure};

use super::CallContext;

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
