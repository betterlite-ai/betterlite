use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::operation::moderation::ModerationFailure;
use betterlite_error::envelope::error_body_call_id;
use betterlite_redact::secret::Redactor;

use crate::routes::chat::headers::failure_headers;
use crate::server::{ErrorBody, ErrorObject};

pub(super) fn render_failure(
    failure: &ModerationFailure,
    call_id: &str,
    include_call_id: bool,
) -> Response {
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = error_body(failure, call_id, include_call_id);
    (
        status,
        failure_headers(None, None, call_id, None),
        Json(body),
    )
        .into_response()
}

fn error_body(failure: &ModerationFailure, call_id: &str, include_call_id: bool) -> ErrorBody {
    ErrorBody {
        error: ErrorObject {
            message: failure.message.clone(),
            error_type: failure.error_type.clone(),
            param: failure.param.clone(),
            code: failure
                .code
                .clone()
                .unwrap_or_else(|| failure.status.to_string()),
            provider_specific_fields: None,
            litellm_call_id: error_body_call_id(include_call_id, Some(call_id)).map(str::to_owned),
        },
    }
}

// the plan's model-less refusal and the router's unknown-model refusal share the 400 shape
pub(super) fn refusal(model: Option<&str>) -> ModerationFailure {
    match model {
        None => ModerationFailure {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: Some(String::from("model")),
            code: Some(String::from("moderation_model_required")),
            message: String::from(
                "the request names no moderation model and general_settings.moderation_model is not set",
            ),
        },
        Some(model) => ModerationFailure {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: None,
            code: None,
            message: format!("no model_list entry serves the model {model}"),
        },
    }
}

pub(super) fn dispatch_failure(
    error: DispatchError,
    call_id: &str,
    include_call_id: bool,
    redactor: &Redactor,
) -> Response {
    let failure = match error {
        DispatchError::Rejected {
            status,
            error_type,
            param,
            code,
            message,
        } => ModerationFailure {
            status,
            error_type: (!error_type.is_empty()).then_some(error_type),
            param,
            code: Some(code),
            message,
        },
        DispatchError::MissingDeployment { model_id } => refusal(Some(&model_id)),
        DispatchError::DeadlineExpired { .. } => ModerationFailure {
            status: 408,
            error_type: None,
            param: None,
            code: None,
            message: String::from("the moderation call passed its deadline"),
        },
        other => ModerationFailure {
            status: 500,
            error_type: Some(String::from("internal_server_error")),
            param: None,
            code: None,
            message: redactor.redact_internal_details(&other.to_string()),
        },
    };
    render_failure(&failure, call_id, include_call_id)
}
