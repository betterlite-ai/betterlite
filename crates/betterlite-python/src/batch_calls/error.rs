use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_read::error::BatchProviderReadError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_openai::batches::records::MissingBatchField;
use betterlite_openai::batches::sdk::TimeoutError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use tokio::task::JoinError;

use super::convert::{ConvertError, SettingsError};
use super::payload::{ErrorPayload, create_error_payload, read_error_payload};

create_exception!(betterlite_native, NativeBatchError, PyException);

// what one provider entry point reports when its executor ran and failed, or the build has none
#[derive(Debug, thiserror::Error)]
pub(crate) enum DispatchFailure {
    #[error("{message}")]
    Status { status: u16, message: String },
    #[error("{0}")]
    Connection(String),
    #[error(transparent)]
    Request(#[from] MissingBatchField),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("retrieving a batch from provider `{provider}` is not available in this build")]
    Unsupported { provider: String },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
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
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json(_) | SessionError::Settings(_) => {
                PyValueError::new_err(error.to_string())
            }
            SessionError::Proxy { .. } | SessionError::Client { .. } => {
                PyRuntimeError::new_err(error.to_string())
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BatchError {
    #[error(transparent)]
    Create(#[from] BatchProviderCreateError),
    #[error(transparent)]
    Read(#[from] BatchProviderReadError<DispatchFailure>),
    #[error("the batch answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch call task failed: {source}")]
    Task {
        #[source]
        source: JoinError,
    },
    #[error("the batch create task ended without a result")]
    TaskWithoutResult,
}

pub(crate) fn payload_for(
    error: &BatchError,
    provider: &str,
    model: Option<&str>,
) -> Option<ErrorPayload> {
    match error {
        BatchError::Create(inner) => Some(create_error_payload(inner, provider, model)),
        BatchError::Read(inner) => Some(read_error_payload(inner, provider, model)),
        BatchError::Encode { .. } | BatchError::Task { .. } | BatchError::TaskWithoutResult => None,
    }
}

// Every failure crosses to Python as NativeBatchError naming its LiteLLM class, except a write failure, which is a runtime error of the session.
pub(crate) fn batch_error(error: &BatchError, provider: &str, model: Option<&str>) -> PyErr {
    let Some(payload) = payload_for(error, provider, model) else {
        return PyRuntimeError::new_err(error.to_string());
    };
    match serde_json::to_string(&payload) {
        Ok(text) => NativeBatchError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(error.to_string()),
    }
}
