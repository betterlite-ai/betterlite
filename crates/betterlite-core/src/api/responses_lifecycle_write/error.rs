use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::call::LifecycleOperation;
use crate::api::responses_write_http::error::WriteError;

// What one entry point can refuse with before or instead of the handler's answer
#[derive(Debug, Error)]
pub enum LifecycleWriteError {
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    #[error("{operation} responses is not supported for {provider}")]
    Unsupported {
        operation: LifecycleOperation,
        provider: String,
    },
    // send() answers with the operation prepare() was given, so the other operation's result means the two disagree
    #[error("the {operation} handler answered with the other operation's result")]
    WrongAnswer { operation: LifecycleOperation },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Transport(#[from] WriteError),
}

// The exception the SDK entry point raises: the typed cause plus the provider exception_type received
#[derive(Debug, Error)]
#[error("{error}")]
pub struct ResponsesLifecycleWriteError {
    #[source]
    pub error: LifecycleWriteError,
    // custom_llm_provider as exception_type receives it; empty while no provider resolved
    pub provider: String,
}

impl ResponsesLifecycleWriteError {
    // LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=cause)
    #[must_use]
    pub fn error_class(&self, redactor: &Redactor) -> ErrorClass {
        let raw = self.raw_exception();
        dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
    }

    fn raw_exception(&self) -> RawException {
        match &self.error {
            // the handler's own exception is what exception_type received
            LifecycleWriteError::Transport(WriteError::Provider(error)) => RawException {
                model: String::new(),
                custom_llm_provider: self.provider.clone(),
                error_str: error.message.clone(),
                status_code: Some(error.status_code),
                status_code_is_synthesized: error.status_code_is_synthesized,
                message: Some(error.message.clone()),
                body: None,
                exception_type: String::new(),
            },
            // a ValueError or a failure that carries no provider status
            LifecycleWriteError::WrongAnswer { .. }
            | LifecycleWriteError::Transport(_)
            | LifecycleWriteError::MissingProvider
            | LifecycleWriteError::Unsupported { .. }
            | LifecycleWriteError::Logging(_) => {
                let message = self.error.to_string();
                RawException {
                    model: String::new(),
                    custom_llm_provider: self.provider.clone(),
                    error_str: message.clone(),
                    status_code: None,
                    status_code_is_synthesized: false,
                    message: Some(message),
                    body: None,
                    exception_type: String::from("ValueError"),
                }
            }
        }
    }
}
