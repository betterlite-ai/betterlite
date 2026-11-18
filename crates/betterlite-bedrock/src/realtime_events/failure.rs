use std::error::Error as StdError;

use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::scope::EventScope;

const BEDROCK_STREAM_ERROR_STATUS: [(&str, u16); 12] = [
    ("AccessDeniedException", 403),
    ("ConflictException", 400),
    ("InternalServerException", 500),
    ("ModelErrorException", 424),
    ("ModelNotReadyException", 429),
    ("ModelStreamErrorException", 424),
    ("ModelTimeoutException", 408),
    ("ResourceNotFoundException", 404),
    ("ServiceQuotaExceededException", 400),
    ("ServiceUnavailableException", 503),
    ("ThrottlingException", 429),
    ("ValidationException", 400),
];

// The seam catches the SDK's own exception type, so the failure names it instead of erasing it
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ProviderFailure<E> {
    pub class_name: String,
    pub message: String,
    #[source]
    pub source: Option<E>,
}

// The mapped error keeps the provider exception as its cause; an unknown one goes up as it arrived
#[derive(Debug, Error)]
pub enum ProviderStreamError<E> {
    #[error("{message}")]
    Bedrock {
        message: String,
        status_code: u16,
        #[source]
        failure: ProviderFailure<E>,
    },
    #[error("{0}")]
    Unchanged(ProviderFailure<E>),
}

#[must_use]
pub fn as_bedrock_error<E>(failure: ProviderFailure<E>) -> ProviderStreamError<E>
where
    E: StdError + 'static,
{
    let mapped_status = BEDROCK_STREAM_ERROR_STATUS
        .iter()
        .find(|(name, _)| *name == failure.class_name.as_str())
        .map(|(_, status)| *status);
    match mapped_status {
        Some(status_code) => ProviderStreamError::Bedrock {
            message: format!("{}: {}", failure.class_name, failure.message),
            status_code,
            failure,
        },
        None => ProviderStreamError::Unchanged(failure),
    }
}

pub fn raise_provider_failure<E>(
    scope: &mut EventScope,
    failure: ProviderFailure<E>,
    redactor: &Redactor,
) -> ProviderStreamError<E>
where
    E: StdError + 'static,
{
    let error = as_bedrock_error(failure);
    let redacted = redactor.redact_string(&error.to_string());
    tracing::error!("Bedrock Realtime: provider stream failed: {redacted}");
    scope.record_failure(redacted);
    error
}
