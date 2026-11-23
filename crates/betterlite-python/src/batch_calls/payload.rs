use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_read::error::BatchProviderReadError;
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

pub(crate) fn create_error_payload(
    error: &BatchProviderCreateError,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    let status = error.status_code();
    // LiteLLM names the provider the request asked for, or n/a when this build has no call for it
    let (target_provider, target_model) = match error {
        BatchProviderCreateError::UnsupportedProvider {
            provider: unsupported,
        } => (unsupported.as_str(), Some("n/a")),
        BatchProviderCreateError::Provider { .. } | BatchProviderCreateError::Limit(_) => {
            (provider, model)
        }
    };
    error_payload(
        class_for_status(status),
        Some(status),
        error.to_string(),
        target_provider,
        target_model,
    )
}

pub(crate) fn read_error_payload(
    error: &BatchProviderReadError<DispatchFailure>,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    match error {
        BatchProviderReadError::Dispatch(failure) => {
            let (error_type, status_code, message) = dispatch_fields(failure);
            error_payload(error_type, status_code, message, provider, model)
        }
        BatchProviderReadError::Limits(limit) => {
            let status = limit.fields().status;
            error_payload(
                class_for_status(status),
                Some(status),
                error.to_string(),
                provider,
                model,
            )
        }
        BatchProviderReadError::Timeout(_)
        | BatchProviderReadError::InvalidVertexCredentials
        | BatchProviderReadError::Unsupported { .. } => error_payload(
            class_for_status(400),
            Some(400),
            error.to_string(),
            provider,
            model,
        ),
    }
}

pub(crate) fn dispatch_fields(failure: &DispatchFailure) -> (&'static str, Option<u16>, String) {
    match failure {
        DispatchFailure::Status { status, message } => {
            (class_for_status(*status), Some(*status), message.clone())
        }
        DispatchFailure::Connection(_) => ("APIConnectionError", None, failure.to_string()),
        DispatchFailure::Request(_)
        | DispatchFailure::Timeout(_)
        | DispatchFailure::Unsupported { .. } => {
            (class_for_status(400), Some(400), failure.to_string())
        }
    }
}
