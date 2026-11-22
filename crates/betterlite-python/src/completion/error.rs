use super::convert::{ConvertError, NativeErrorPayload};
use betterlite_env::EnvError;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::error::HttpError;
use betterlite_sdk::call_wrapper::error::CallError;
use betterlite_tokenizer::count::error::CountError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
create_exception!(betterlite_native, NativeError, PyException);
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the tokenizer counter could not be built: {source}")]
    Tokenizer {
        #[source]
        source: CountError,
    },
}
pub(crate) fn call_error(error: &CallError) -> PyErr {
    let payload = NativeErrorPayload::from_call_error(error);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}
impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        match &error {
            ConvertError::Json { .. } => PyValueError::new_err(error.to_string()),
            ConvertError::Encode { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}
impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json(_) => PyValueError::new_err(error.to_string()),
            SessionError::Proxy { .. }
            | SessionError::Client { .. }
            | SessionError::Tokenizer { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}
// main.py's valid_model raises BadRequestError(message="", model=model, llm_provider="") for every failure
pub(crate) fn validation_error(model: &str) -> PyErr {
    let details = ResponseDetails::<400>::from(ErrorContext {
        message: String::new(),
        model: Some(model.to_owned()),
        provider: Some(String::new()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    });
    call_error(&CallError::from(ErrorClass::BadRequest(Box::new(details))))
}
