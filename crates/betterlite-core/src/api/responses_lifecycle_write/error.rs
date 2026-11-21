use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::responses_write_http::error::WriteError;

// What one entry point can refuse with before or instead of the handler's answer
#[derive(Debug, Error)]
pub enum LifecycleWriteError {
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    // the label is the operation as LiteLLM's ValueError spells it
    #[error("{label} responses is not supported for {provider}")]
    Unsupported {
        label: &'static str,
        provider: String,
    },
    // send() answers with the operation prepare() was given, so the other operation's result means the two disagree
    #[error("the {label} handler answered with the other operation's result")]
    WrongAnswer { label: &'static str },
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
    // LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=cause): a success reply the transform could not decode carries the reply status, which the status branch maps to APIError
    #[must_use]
    pub fn error_class(&self, redactor: &Redactor) -> ErrorClass {
        let (status_code, synthesized, exception_type) = match &self.error {
            LifecycleWriteError::Transport(WriteError::Provider(error)) => (
                Some(error.status_code),
                error.status_code_is_synthesized,
                String::new(),
            ),
            LifecycleWriteError::Transport(WriteError::Resource(error)) => {
                (error.http_status(), false, String::from("OpenAIError"))
            }
            LifecycleWriteError::WrongAnswer { .. }
            | LifecycleWriteError::Transport(_)
            | LifecycleWriteError::MissingProvider
            | LifecycleWriteError::Unsupported { .. }
            | LifecycleWriteError::Logging(_) => (None, false, String::from("ValueError")),
        };
        let message = self.error.to_string();
        dispatch(
            &DispatchInput::Raw(Box::new(RawException {
                model: String::new(),
                custom_llm_provider: self.provider.clone(),
                error_str: message.clone(),
                status_code,
                status_code_is_synthesized: synthesized,
                message: Some(message),
                body: None,
                exception_type,
            })),
            redactor,
        )
    }
}
