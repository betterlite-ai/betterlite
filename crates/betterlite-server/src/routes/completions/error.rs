use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_openai::chat::deployment::ChatDeployment;
use serde::Serialize;

use crate::routes::chat::headers::failure_headers;

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

pub(super) struct FailureAnswer {
    pub(super) status: u16,
    pub(super) error_type: String,
    pub(super) param: Option<String>,
    pub(super) message: String,
}

impl FailureAnswer {
    pub(super) fn request(message: String) -> Self {
        Self {
            status: 400,
            error_type: String::from("invalid_request_error"),
            param: None,
            message,
        }
    }

    fn internal(message: &str) -> Self {
        Self {
            status: 500,
            error_type: String::from("internal_server_error"),
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
            error_type,
            param,
            message,
        },
        DispatchError::MissingDeployment { model_id } => FailureAnswer {
            status: 400,
            error_type: String::from("invalid_request_error"),
            param: None,
            message: format!("no model_list entry serves the model {model_id}"),
        },
        DispatchError::DeadlineExpired { .. } => FailureAnswer {
            status: 408,
            error_type: String::from("invalid_request_error"),
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

pub(super) fn render(
    answer: &FailureAnswer,
    upstream: Option<&HeaderMap>,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
) -> Response {
    let status_code =
        StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorBody {
        error: ErrorObject {
            message: answer.message.clone(),
            error_type: answer.error_type.clone(),
            param: answer.param.clone(),
            code: answer.status.to_string(),
        },
    };
    (
        status_code,
        failure_headers(None, upstream, call_id, deployment),
        Json(body),
    )
        .into_response()
}
