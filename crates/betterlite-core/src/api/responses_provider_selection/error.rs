use super::provider_config::CompatibilityFailure;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_redact::secret::Redactor;
use thiserror::Error;
pub const ENCRYPTED_TASK_MESSAGE: &str =
    "Encrypted task classification requires a compatible native Responses deployment";
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
        error_str: String::from(match failure {
            CompatibilityFailure::EncryptedTaskUnsupported => ENCRYPTED_TASK_MESSAGE,
        }),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}
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
