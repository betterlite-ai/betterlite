use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::messages_responses_output::error::MessagesResponsesOutputError;
use crate::api::responses_create::error::{CreateError, CreateFailure};

// The exceptions the handler raises before any HTTP answer exists; normalize passes each through B014's exception_type mapping.
#[derive(Debug, Error)]
pub enum MessagesResponseCallError {
    #[error(transparent)]
    Build(#[from] CreateError),
    #[error(transparent)]
    Translate(#[from] MessagesResponsesOutputError),
    #[error("Expected ResponsesAPIResponse, got {kind}")]
    Unexpected { kind: String },
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("{}", .0.message)]
    Create(CreateFailure),
}

impl MessagesResponseCallError {
    #[must_use]
    pub fn normalize(&self, model: &str, provider: &str, redactor: &Redactor) -> CreateFailure {
        match self {
            Self::Build(error) => error.sdk_failure(provider, model, redactor),
            Self::Translate(error) => value_failure(&error.to_string(), model, provider, redactor),
            Self::Unexpected { kind } => value_failure(
                &format!("Expected ResponsesAPIResponse, got {kind}"),
                model,
                provider,
                redactor,
            ),
            Self::Resolution(error) => resolution_failure(error, model, redactor),
            Self::Create(failure) => failure.clone(),
        }
    }
}

// A bare ValueError carries no status, so exception_type's last resort answers APIConnectionError with bounded text.
fn value_failure(message: &str, model: &str, provider: &str, redactor: &Redactor) -> CreateFailure {
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    };
    class_failure(
        &dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        redactor,
    )
}

// get_llm_provider wraps every failure of its try block in a BadRequestError with the original model appended.
fn resolution_failure(error: &ResolutionError, model: &str, redactor: &Redactor) -> CreateFailure {
    let text = format!("GetLLMProvider Exception - {error}\n\noriginal model: {model}");
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: String::new(),
        error_str: text.clone(),
        status_code: Some(400),
        status_code_is_synthesized: false,
        message: Some(text),
        body: None,
        exception_type: String::from("BadRequestError"),
    };
    class_failure(
        &dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        redactor,
    )
}

// Every class built here reaches the caller with its kind's public prefix, as exception_type prints it.
fn class_failure(class: &ErrorClass, redactor: &Redactor) -> CreateFailure {
    CreateFailure {
        class: class_name(class),
        status: class.metadata().status_code,
        kind: None,
        param: None,
        code: None,
        message: redactor.redact_string(&class_message(class)),
    }
}
