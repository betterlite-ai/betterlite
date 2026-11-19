use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response as AxumResponse};
use betterlite_error::envelope::error_body_call_id;
use betterlite_openai::chat::deployment::ChatDeployment;

use crate::routes::chat::CallContext;
use crate::routes::chat::headers::failure_headers;
use crate::server::{ErrorBody, ErrorObject, RouterErrorFields};

pub(crate) struct FailureShape<'kind> {
    pub(crate) status: StatusCode,
    pub(crate) error_type: &'kind str,
    pub(crate) param: Option<String>,
    pub(crate) message: String,
    // LiteLLM's router repeats its message in provider_specific_fields for a model-not-found
    pub(crate) repeat_message: bool,
}

pub(crate) fn plain_failure(
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
    shape: FailureShape<'_>,
) -> AxumResponse {
    let FailureShape {
        status,
        error_type,
        param,
        message,
        repeat_message,
    } = shape;
    let provider_specific_fields = if repeat_message {
        Some(RouterErrorFields {
            error: message.clone(),
        })
    } else {
        None
    };
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: Some(error_type.to_owned()),
            param,
            code: status.as_u16().to_string(),
            provider_specific_fields,
            litellm_call_id: error_body_call_id(call.include_in_body, Some(call.id))
                .map(str::to_owned),
        },
    };
    let headers = failure_headers(None, None, call.id, deployment);
    (status, headers, Json(body)).into_response()
}
