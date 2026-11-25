use super::convert::{ConvertError, NativeRerankErrorPayload};
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_core::api::rerank_provider_plan::plan::RerankException;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};

create_exception!(betterlite_native, NativeRerankError, PyException);

// One failed call: the session could not be built, or the rerank dispatcher raised its exception.
#[derive(Debug)]
pub(crate) enum CallFailure {
    Session(SessionError),
    Rerank(RerankException),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error("the redactor could not be built: {source}")]
    Redact {
        #[from]
        source: RedactionError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
}

// NativeRerankError carries the payload main.py maps to LiteLLM's exception class; a session error is a plain one.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn call_failure(failure: CallFailure) -> PyErr {
    match failure {
        CallFailure::Session(error) => PyErr::from(error),
        CallFailure::Rerank(failure) => class_error(&failure.class),
    }
}

#[cfg_attr(test, mutants::skip)]
fn class_error(class: &ErrorClass) -> PyErr {
    let payload = NativeRerankErrorPayload::from_class(class);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeRerankError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

impl From<SessionError> for CallFailure {
    #[cfg_attr(test, mutants::skip)]
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<RerankException> for CallFailure {
    #[cfg_attr(test, mutants::skip)]
    fn from(failure: RerankException) -> Self {
        Self::Rerank(failure)
    }
}

impl From<SessionError> for PyErr {
    #[cfg_attr(test, mutants::skip)]
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) => PyValueError::new_err(message),
            SessionError::Json(ConvertError::Encode { .. })
            | SessionError::Completion(_)
            | SessionError::Redact { .. }
            | SessionError::Logging(_) => PyRuntimeError::new_err(message),
        }
    }
}
