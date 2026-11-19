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
    let body = ErrorBody {
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
    };
    (
        status,
        failure_headers(None, None, call_id, None),
        Json(body),
    )
        .into_response()
}

// the plan's model-less refusal and the router's unknown-model refusal share the 400 shape
pub(super) fn refusal(model: Option<&str>) -> ModerationFailure {
    model.map_or_else(
        || ModerationFailure {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: Some(String::from("model")),
            code: Some(String::from("moderation_model_required")),
            message: String::from(
                "the request names no moderation model and general_settings.moderation_model is not set",
            ),
        },
        |served| ModerationFailure {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: None,
            code: None,
            message: format!("no model_list entry serves the model {served}"),
        },
    )
}

pub(super) fn dispatch_failure(error: DispatchError, redactor: &Redactor) -> ModerationFailure {
    match error {
        DispatchError::MissingDeployment { model_id } => refusal(Some(&model_id)),
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
        DispatchError::DeadlineExpired { .. } => {
            ModerationFailure::plain(408, None, "the moderation call passed its deadline")
        }
        other @ (DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. }
        | DispatchError::Rules { .. }
        | DispatchError::Chat { .. }
        | DispatchError::Embedding { .. }) => ModerationFailure::plain(
            500,
            Some("internal_server_error"),
            &redactor.redact_internal_details(&other.to_string()),
        ),
    }
}
