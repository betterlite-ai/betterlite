use betterlite_core::api::batch_provider_cancel::BatchProviderCancelError;
use betterlite_core::api::batch_provider_list::error::BatchProviderListError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_openai::batches::sdk::TimeoutError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

use super::convert::{ConvertError, SettingsError};
use super::payload::{
    ErrorPayload, ProviderError, dispatch_fields, error_payload, provider_error_payload,
};

create_exception!(betterlite_native, NativeBatchListError, PyException);

// The three fields the openai SDK's classes read off a provider error object; the rest of the body is not on the native error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ErrorBody {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
}

impl ErrorBody {
    // a status whose error object names none of the three fields has no body to hand on
    pub(crate) fn from_fields(
        error_type: Option<String>,
        param: Option<String>,
        code: Option<String>,
    ) -> Option<Self> {
        if error_type.is_none() && param.is_none() && code.is_none() {
            return None;
        }
        Some(Self {
            error_type,
            param,
            code,
        })
    }
}

// what one provider entry point reports when its executor ran and failed, or the build has none
#[derive(Debug, thiserror::Error)]
pub(crate) enum DispatchFailure {
    #[error("{message}")]
    Status {
        status: u16,
        message: String,
        body: Option<ErrorBody>,
    },
    #[error("{0}")]
    Connection(String),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("provider `{provider}` has no batch listing or cancel call in this build")]
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
pub(crate) enum ListingError {
    #[error(transparent)]
    List(#[from] BatchProviderListError),
    #[error(transparent)]
    Cancel(#[from] BatchProviderCancelError<DispatchFailure>),
    #[error("the batch provider call failed: {0}")]
    Dispatch(#[from] DispatchFailure),
    #[error("the batch answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn payload_for(
    error: &ListingError,
    provider: &str,
    model: Option<&str>,
) -> Option<ErrorPayload> {
    match error {
        ListingError::List(inner) => Some(provider_error_payload(
            ProviderError::List(inner),
            provider,
            model,
        )),
        ListingError::Cancel(inner) => Some(provider_error_payload(
            ProviderError::Cancel(inner),
            provider,
            model,
        )),
        ListingError::Dispatch(failure) => {
            Some(error_payload(dispatch_fields(failure), provider, model))
        }
        ListingError::Encode { .. } => None,
    }
}

// Every failure crosses to Python as NativeBatchListError naming its LiteLLM class, except a write failure, which is a runtime error of the session.
pub(crate) fn listing_error(error: &ListingError, provider: &str, model: Option<&str>) -> PyErr {
    let Some(payload) = payload_for(error, provider, model) else {
        return PyRuntimeError::new_err(error.to_string());
    };
    match serde_json::to_string(&payload) {
        Ok(text) => NativeBatchListError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(error.to_string()),
    }
}
