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

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ProviderFailure {
    pub class_name: String,
    pub message: String,
}

// The mapped error keeps the provider exception as its cause; an unknown one goes up as it arrived
#[derive(Debug, Error)]
pub enum ProviderStreamError {
    #[error("{message}")]
    Bedrock {
        message: String,
        status_code: u16,
        #[source]
        failure: ProviderFailure,
    },
    #[error(transparent)]
    Unchanged(#[from] ProviderFailure),
}

#[must_use]
pub fn as_bedrock_error(failure: ProviderFailure) -> ProviderStreamError {
    let status_code = BEDROCK_STREAM_ERROR_STATUS
        .iter()
        .find(|(name, _)| *name == failure.class_name.as_str())
        .map(|(_, status)| *status);
    match status_code {
        Some(status_code) => ProviderStreamError::Bedrock {
            message: format!("{}: {}", failure.class_name, failure.message),
            status_code,
            failure,
        },
        None => ProviderStreamError::Unchanged(failure),
    }
}

pub fn raise_provider_failure(
    scope: &mut EventScope,
    failure: ProviderFailure,
    redactor: &Redactor,
) -> ProviderStreamError {
    let error = as_bedrock_error(failure);
    let redacted = redactor.redact_string(&error.to_string());
    tracing::error!("Bedrock Realtime: provider stream failed: {redacted}");
    scope.record_failure(redacted);
    error
}
