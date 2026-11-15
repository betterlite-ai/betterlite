use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_error::envelope::error_body_call_id;

use crate::routes::chat::headers::failure_headers;
use crate::server::{ErrorBody, ErrorObject};

pub(super) struct FailureAnswer {
    pub(super) status: u16,
    pub(super) error_type: Option<String>,
    pub(super) param: Option<String>,
    pub(super) message: String,
}

impl FailureAnswer {
    pub(super) fn request(message: String) -> Self {
        Self {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: None,
            message,
        }
    }

    fn internal(message: &str) -> Self {
        Self {
            status: 500,
            error_type: Some(String::from("internal_server_error")),
            param: None,
            message: String::from(message),
        }
    }
}

pub(super) fn failure_answer(error: DispatchError) -> FailureAnswer {
    match error {
        DispatchError::Rejected {
            status,
            error_type,
            param,
            message,
            ..
        } => FailureAnswer {
            status,
            // the text completion dispatch keeps a null type as an empty string, so it serializes as null again
            error_type: (!error_type.is_empty()).then_some(error_type),
            param,
            message,
        },
        DispatchError::MissingDeployment { model_id } => FailureAnswer {
            status: 400,
            error_type: Some(String::from("invalid_request_error")),
            param: None,
            message: format!("no model_list entry serves the model {model_id}"),
        },
        DispatchError::DeadlineExpired { .. } => FailureAnswer {
            status: 408,
            error_type: Some(String::from("invalid_request_error")),
            param: None,
            message: String::from("the completion call passed its deadline"),
        },
        DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. }
        | DispatchError::Rules { .. }
        | DispatchError::Chat { .. }
        | DispatchError::Embedding { .. } => {
            FailureAnswer::internal("the completion call could not be dispatched")
        }
    }
}

// proxy_server.py's completion except answers a failure with litellm_call_id_headers alone, so no provider or deployment header comes back
pub(super) fn render(answer: &FailureAnswer, call_id: &str, include_call_id: bool) -> Response {
    let status = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorBody {
        error: ErrorObject {
            message: answer.message.clone(),
            error_type: answer.error_type.clone(),
            param: answer.param.clone(),
            code: answer.status.to_string(),
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
