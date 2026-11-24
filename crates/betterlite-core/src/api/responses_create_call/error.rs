use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::decisions_send::error::visible_message;
use crate::api::responses_create::error::{CreateError, CreateFailure};
use crate::api::responses_provider_selection::error::CredentialBoundaryError;

// The causes aresponses raises itself, each reaching exception_type with its own class
#[derive(Debug, Error)]
pub enum AsyncCreateCause {
    #[error(transparent)]
    Create(#[from] CreateError),
    #[error("{0}")]
    CredentialBoundary(CredentialBoundaryError),
    // get_llm_provider raises BadRequestError before any HTTP, and exception_type hands it back unchanged
    #[error("{0}")]
    ProviderResolution(ResolutionError),
    #[error("Got an unexpected None response from the Responses API: None")]
    NullResponse,
}

// What the caller sees, attributed to the model and provider LiteLLM's except block reads off its locals
#[derive(Debug, Error)]
#[error("{cause}")]
pub struct AsyncCreateError {
    pub cause: Box<AsyncCreateCause>,
    pub provider: String,
    pub model: String,
}

impl AsyncCreateError {
    // LiteLLM raises BadRequestError(model=swapped, llm_provider=swapped) and exception_type hands it back unchanged
    pub(super) fn boundary(error: CredentialBoundaryError) -> Self {
        let provider = error.provider.clone();
        let model = error.model.clone();
        Self {
            cause: Box::new(AsyncCreateCause::CredentialBoundary(error)),
            provider,
            model,
        }
    }

    #[must_use]
    pub fn sdk_failure(&self, redactor: &Redactor) -> CreateFailure {
        match self.cause.as_ref() {
            AsyncCreateCause::Create(error) => {
                error.sdk_failure(&self.provider, &self.model, redactor)
            }
            AsyncCreateCause::CredentialBoundary(error) => class_failure(&error.class(), redactor),
            AsyncCreateCause::ProviderResolution(error) => {
                let class = provider_resolution_class(error, &self.model);
                class_failure(&class, redactor)
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

// Every class built here reaches the caller with its kind's public prefix, as exception_type prints it
fn class_failure(class: &ErrorClass, redactor: &Redactor) -> CreateFailure {
    CreateFailure {
        class: class_name(class),
        status: class.metadata().status_code,
        kind: None,
        param: None,
        code: None,
        message: redactor.redact_string(&visible_message(class)),
    }
}

// get_llm_provider's BadRequestError names the model and leaves the provider empty, since neither local is filled yet
fn provider_resolution_class(error: &ResolutionError, model: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
        ErrorContext {
            message: error.to_string(),
            model: Some(model.to_owned()),
            provider: Some(String::new()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        None,
    )))
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
