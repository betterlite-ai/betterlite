use betterlite_core::api::fine_tuning_cancel_call::FineTuningCancelError;

use super::dispatch::CancelFailure;
use crate::sdk_http::result::{Failure, Nullable};

// the class each failure projects to; only the SDK's status errors carry a status, type, param and code
pub(super) fn failure(error: &FineTuningCancelError<CancelFailure>) -> Failure {
    match error {
        FineTuningCancelError::Unsupported { .. } => {
            null_failure("BadRequestError", 400, error.to_string())
        }
        FineTuningCancelError::Timeout(_)
        | FineTuningCancelError::Dispatch(
            CancelFailure::MissingBase | CancelFailure::InvalidHeader,
        ) => null_failure("ValueError", 0, error.to_string()),
        FineTuningCancelError::Dispatch(CancelFailure::MissingKey) => {
            null_failure("OpenAIError", 0, error.to_string())
        }
        FineTuningCancelError::Dispatch(CancelFailure::Status { status, body }) => {
            status_failure(*status, body)
        }
        FineTuningCancelError::Dispatch(CancelFailure::StatusUtf8 { status, .. }) => {
            null_failure(status_class(*status), *status, error.to_string())
        }
        FineTuningCancelError::Dispatch(
            CancelFailure::Url(_) | CancelFailure::Transport(_) | CancelFailure::Decode(_),
        ) => null_failure("APIConnectionError", 0, error.to_string()),
    }
}

// an error the SDK raises before or without a provider body leaves type, param and code null
fn null_failure(class: &str, status: u16, message: String) -> Failure {
    Failure {
        class: class.to_owned(),
        status,
        kind: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    }
}

// a status error reads type, param and code out of the body and renders the status with the raw body
fn status_failure(status: u16, body: &str) -> Failure {
    let (kind, param, code) = body_fields(body);
    Failure {
        class: status_class(status).to_owned(),
        status,
        kind,
        param,
        code,
        message: format!("Error code: {status} - {body}"),
    }
}

const fn status_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        _ if status >= 500 => "InternalServerError",
        _ => "APIStatusError",
    }
}

// the SDK reads type, param and code from the body, or from its "error" member when it has one
fn body_fields(body: &str) -> (Nullable, Nullable, Nullable) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let candidate = match &parsed {
        serde_json::Value::Object(fields) => fields.get("error").unwrap_or(&parsed),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)
        | serde_json::Value::Array(_) => &parsed,
    };
    let serde_json::Value::Object(fields) = candidate else {
        return (Some(None), Some(None), Some(None));
    };
    (
        Some(nullable(fields.get("type"))),
        Some(nullable(fields.get("param"))),
        Some(nullable(fields.get("code"))),
    )
}

fn nullable(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(other) => Some(other.to_string()),
    }
}
