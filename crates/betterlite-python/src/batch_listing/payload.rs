use betterlite_core::api::batch_provider_cancel::BatchProviderCancelError;
use betterlite_core::api::batch_provider_list::error::BatchProviderListError;
use serde::Serialize;

use super::error::DispatchFailure;

// What _completion/errors.py builds a LiteLLM exception from: the class name, the message and the status.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ErrorPayload {
    #[serde(rename = "type")]
    pub(crate) error_type: &'static str,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: String,
    pub(crate) model: Option<String>,
}

pub(crate) const fn class_for_status(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIError",
    }
}

fn error_payload(
    error_type: &'static str,
    status_code: Option<u16>,
    message: String,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    ErrorPayload {
        error_type,
        message,
        status_code,
        llm_provider: provider.to_owned(),
        model: model.map(str::to_owned),
    }
}

// LiteLLM names the provider the request asked for and model n/a when its branch table has no entry
pub(crate) fn list_error_payload(error: &BatchProviderListError, provider: &str) -> ErrorPayload {
    let (target_provider, target_model) = match error {
        BatchProviderListError::UnsupportedProvider {
            provider: unsupported,
        } => (unsupported.as_str(), Some("n/a")),
        BatchProviderListError::InvalidTimeout { .. }
        | BatchProviderListError::NonFiniteTimeout
        | BatchProviderListError::InvalidVertexCredentials => (provider, None),
    };
    error_payload(
        class_for_status(400),
        Some(400),
        error.to_string(),
        target_provider,
        target_model,
    )
}

pub(crate) fn cancel_error_payload(
    error: &BatchProviderCancelError<DispatchFailure>,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    match error {
        BatchProviderCancelError::Dispatch(failure) => {
            let (error_type, status_code, message) = dispatch_fields(failure);
            error_payload(error_type, status_code, message, provider, model)
        }
        BatchProviderCancelError::Limits(limit) => {
            let status = limit.fields().status;
            error_payload(
                class_for_status(status),
                Some(status),
                error.to_string(),
                provider,
                model,
            )
        }
        BatchProviderCancelError::Timeout(_) => error_payload(
            class_for_status(400),
            Some(400),
            error.to_string(),
            provider,
            model,
        ),
        BatchProviderCancelError::Unsupported {
            provider: unsupported,
        } => error_payload(
            class_for_status(400),
            Some(400),
            error.to_string(),
            unsupported,
            Some("n/a"),
        ),
    }
}

pub(crate) fn dispatch_fields(failure: &DispatchFailure) -> (&'static str, Option<u16>, String) {
    match failure {
        DispatchFailure::Status { status, message } => {
            (class_for_status(*status), Some(*status), message.clone())
        }
        DispatchFailure::Connection(_) => ("APIConnectionError", None, failure.to_string()),
        DispatchFailure::Timeout(_) | DispatchFailure::Unsupported { .. } => {
            (class_for_status(400), Some(400), failure.to_string())
        }
    }
}

// a failure the transport answered with, under the provider and model the payload asked for
pub(crate) fn dispatch_payload(
    failure: &DispatchFailure,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    let (error_type, status_code, message) = dispatch_fields(failure);
    error_payload(error_type, status_code, message, provider, model)
}
