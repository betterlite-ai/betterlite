use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::error::FineTuneError;
use http::header::InvalidHeaderValue;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use serde::Serialize;

create_exception!(betterlite_native, NativeTuningError, PyException);

// The JSON text NativeTuningError carries: the LiteLLM exception name and the fields its class keeps.
#[derive(Debug, Serialize)]
pub(crate) struct Failure {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WireError {
    #[error("the provider answer is not JSON of the expected shape: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the fine-tuning request body could not be written: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the fine-tuning job record is not valid: {source}")]
    Decode {
        #[source]
        source: FineTuneError,
    },
    #[error("the provider request URL is not valid: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the provider request header {name} is not valid: {source}")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the provider answered with status {status}")]
    Provider {
        status: u16,
        body: Json,
        headers: BTreeMap<String, String>,
    },
    #[error("the provider request failed: {source}")]
    Transport {
        #[source]
        source: Box<TransportError>,
    },
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the fine-tuning settings could not be read: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the openai fine-tuning client needs an api key")]
    MissingOpenAiKey,
    #[error("the azure fine-tuning client needs an api_base")]
    MissingAzureBase,
    #[error("the azure fine-tuning client needs an api key")]
    MissingAzureKey,
    #[error("the vertex_ai fine-tuning call is not implemented yet")]
    Vertex,
    #[error(transparent)]
    JobId(#[from] HttpError),
}

// What a native method raises: a LiteLLM exception described as JSON, or a plain Python error.
#[derive(Debug)]
pub(crate) enum Raised {
    Failure(Box<Failure>),
    Python(Box<PyErr>),
}

impl Raised {
    pub(crate) fn into_pyerr(self) -> PyErr {
        match self {
            Self::Failure(failure) => match serde_json::to_string(&failure) {
                Ok(text) => NativeTuningError::new_err(text),
                Err(_encoding) => PyRuntimeError::new_err(failure.message),
            },
            Self::Python(error) => *error,
        }
    }
}
