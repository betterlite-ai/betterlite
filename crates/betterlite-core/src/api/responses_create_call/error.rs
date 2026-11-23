use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::decisions_send::error::visible_message;
use crate::api::responses_create::error::{CreateError, CreateFailure};
use crate::api::responses_provider_selection::error::CredentialBoundaryError;

// The local cause LiteLLM's aresponses raises for a null answer, raised again as a bare ValueError
#[derive(Debug, Error)]
pub enum AsyncCreateCause {
    #[error(transparent)]
    Create(#[from] CreateError),
    #[error("{0}")]
    CredentialBoundary(CredentialBoundaryError),
    #[error("Got an unexpected None response from the Responses API: None")]
    NullResponse,
}

// What the caller sees, attributed to the model and provider LiteLLM's except block reads off its locals
#[derive(Debug, Error)]
#[error("{cause}")]
pub struct AsyncCreateError {
    pub cause: AsyncCreateCause,
    pub provider: String,
    pub model: String,
}

impl AsyncCreateError {
    pub(super) fn create(error: CreateError, provider: &str, model: &str) -> Self {
        Self {
            cause: AsyncCreateCause::Create(error),
            provider: provider.to_owned(),
            model: model.to_owned(),
        }
    }

    // LiteLLM raises BadRequestError(model=swapped, llm_provider=swapped) and exception_type hands it back unchanged
    pub(super) fn boundary(error: CredentialBoundaryError) -> Self {
        Self {
            provider: error.provider.clone(),
            model: error.model.clone(),
            cause: AsyncCreateCause::CredentialBoundary(error),
        }
    }

    pub(super) fn null_response(provider: String, model: String) -> Self {
        Self {
            cause: AsyncCreateCause::NullResponse,
            provider,
            model,
        }
    }

    #[must_use]
    pub fn sdk_failure(&self, redactor: &Redactor) -> CreateFailure {
        match &self.cause {
            AsyncCreateCause::Create(error) => {
                error.sdk_failure(&self.provider, &self.model, redactor)
            }
            AsyncCreateCause::CredentialBoundary(error) => {
                let class = error.class();
                CreateFailure {
                    class: class_name(&class),
                    status: class.metadata().status_code,
                    kind: None,
                    param: None,
                    code: None,
                    message: redactor.redact_string(&visible_message(&class)),
                }
            }
            AsyncCreateCause::NullResponse => {
                let class = null_response_class(&self.provider, &self.model, redactor);
                CreateFailure {
                    class: class_name(&class),
                    status: class.metadata().status_code,
                    kind: None,
                    param: None,
                    code: None,
                    message: redactor.redact_string(&class_message(&class)),
                }
            }
        }
    }
}

// A bare ValueError has no status, so exception_type's last resort answers APIConnectionError with bounded text
fn null_response_class(provider: &str, model: &str, redactor: &Redactor) -> ErrorClass {
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: AsyncCreateCause::NullResponse.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}
