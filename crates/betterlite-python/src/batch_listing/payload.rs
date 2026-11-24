use betterlite_core::api::batch_provider_cancel::BatchProviderCancelError;
use betterlite_core::api::batch_provider_list::error::BatchProviderListError;
use serde::Serialize;

use super::error::{DispatchFailure, ErrorBody};

// What _completion/errors.py builds a LiteLLM exception from: the class name, the message, the status and the SDK's error body.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ErrorPayload {
    #[serde(rename = "type")]
    pub(crate) error_type: &'static str,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: String,
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<ErrorBody>,
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

// the class, status, message and body one failure contributes to the envelope
pub(crate) struct PayloadFields {
    error_type: &'static str,
    status_code: Option<u16>,
    message: String,
    body: Option<ErrorBody>,
}

// a status whose error object is not the reason: a locally refused call or a limit the core hit
const fn status_fields(status: u16, message: String) -> PayloadFields {
    PayloadFields {
        error_type: class_for_status(status),
        status_code: Some(status),
        message,
        body: None,
    }
}

pub(crate) fn error_payload(
    fields: PayloadFields,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    ErrorPayload {
        error_type: fields.error_type,
        message: fields.message,
        status_code: fields.status_code,
        llm_provider: provider.to_owned(),
        model: model.map(str::to_owned),
        body: fields.body,
    }
}

// the provider error a list or cancel call answered with, in the entry's own type
#[derive(Clone, Copy)]
pub(crate) enum ProviderError<'error> {
    List(&'error BatchProviderListError),
    Cancel(&'error BatchProviderCancelError<DispatchFailure>),
}

// LiteLLM names the provider the request asked for and model n/a when its branch table has no entry
pub(crate) fn provider_error_payload(
    error: ProviderError<'_>,
    provider: &str,
    model: Option<&str>,
) -> ErrorPayload {
    match error {
        ProviderError::List(failure) => {
            let (named_provider, named_model) = list_target(failure, provider);
            error_payload(
                status_fields(400, failure.to_string()),
                named_provider,
                named_model,
            )
        }
        ProviderError::Cancel(failure) => match failure {
            BatchProviderCancelError::Dispatch(dispatch) => {
                error_payload(dispatch_fields(dispatch), provider, model)
            }
            BatchProviderCancelError::Limits(limit) => {
                let status = limit.fields().status;
                error_payload(status_fields(status, failure.to_string()), provider, model)
            }
            BatchProviderCancelError::Timeout(_) => {
                error_payload(status_fields(400, failure.to_string()), provider, model)
            }
            BatchProviderCancelError::Unsupported {
                provider: unsupported,
            } => error_payload(
                status_fields(400, failure.to_string()),
                unsupported.as_str(),
                Some("n/a"),
            ),
        },
    }
}

// the provider and model a list error names: the unsupported one and n/a, or the caller's and none
const fn list_target<'error>(
    error: &'error BatchProviderListError,
    provider: &'error str,
) -> (&'error str, Option<&'error str>) {
    match error {
        BatchProviderListError::UnsupportedProvider {
            provider: unsupported,
        } => (unsupported.as_str(), Some("n/a")),
        BatchProviderListError::InvalidTimeout { .. }
        | BatchProviderListError::NonFiniteTimeout
        | BatchProviderListError::InvalidVertexCredentials => (provider, None),
    }
}

pub(crate) fn dispatch_fields(failure: &DispatchFailure) -> PayloadFields {
    match failure {
        DispatchFailure::Status {
            status,
            message,
            body,
        } => PayloadFields {
            error_type: class_for_status(*status),
            status_code: Some(*status),
            message: message.clone(),
            body: body.clone(),
        },
        DispatchFailure::Connection(_) => PayloadFields {
            error_type: "APIConnectionError",
            status_code: None,
            message: failure.to_string(),
            body: None,
        },
        DispatchFailure::Timeout(_) | DispatchFailure::Unsupported { .. } => {
            status_fields(400, failure.to_string())
        }
    }
}
