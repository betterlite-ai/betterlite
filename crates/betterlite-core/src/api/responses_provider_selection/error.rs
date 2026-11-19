use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::provider_config::CompatibilityFailure;

pub const ENCRYPTED_TASK_MESSAGE: &str =
    "Encrypted task classification requires a compatible native Responses deployment";

// LiteLLM's _raise_responses_compatibility_failure: a ValueError goes through exception_type()
#[must_use]
pub fn compatibility_failure(
    failure: CompatibilityFailure,
    model: &str,
    provider: Option<&str>,
    redactor: &Redactor,
) -> ErrorClass {
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.unwrap_or_default().to_owned(),
        error_str: failure_message(failure).to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: "ValueError".to_owned(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn failure_message(failure: CompatibilityFailure) -> &'static str {
    match failure {
        CompatibilityFailure::EncryptedTaskUnsupported => ENCRYPTED_TASK_MESSAGE,
    }
}

// LiteLLM's BadRequestError built at the prompt swap boundary: status 400, no body fields
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CredentialBoundaryError {
    pub message: String,
    pub model: String,
    pub provider: String,
}

impl CredentialBoundaryError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
            ErrorContext {
                message: self.message.clone(),
                model: Some(self.model.clone()),
                provider: Some(self.provider.clone()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            },
            None,
        )))
    }
}
