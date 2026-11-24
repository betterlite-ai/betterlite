use betterlite_config::loader::ConfigError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_models::catalog::CatalogError;
use betterlite_models::error::ModelsError;
use betterlite_models::model_metadata_resolution::error::{ADD_MODEL_URL, ModelMetadataError};
use betterlite_models::model_metadata_resolution::names::ModelCostMapError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use std::time::TryFromFloatSecsError;
create_exception!(betterlite_native, ModelLookupError, PyException);
// The native methods answer with one error type; an unmapped model is the only one Python raises as ModelNotMappedError.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ModelInfoError {
    #[error(transparent)]
    Lookup(#[from] ModelMetadataError),
    #[error("Model {model} isn't mapped yet. Add it here - {ADD_MODEL_URL}")]
    MaxTokens { model: String },
    #[error("invalid request JSON: {0}")]
    Request(#[from] serde_json::Error),
    #[error("model cost map: {0}")]
    Map(#[from] ModelCostMapError),
    #[error("model catalog: {0}")]
    Catalog(#[from] CatalogError),
    #[error("fallback generalizations: {0}")]
    Generalizations(#[from] ModelsError),
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
    #[error("the model cost map settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
    #[error("timeout {timeout} is not a duration")]
    Timeout {
        timeout: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("result JSON: {0}")]
    Result(#[source] serde_json::Error),
}
impl From<ModelInfoError> for PyErr {
    fn from(error: ModelInfoError) -> Self {
        let message = error.to_string();
        match error {
            ModelInfoError::Lookup(_) | ModelInfoError::MaxTokens { .. } => {
                ModelLookupError::new_err(message)
            }
            ModelInfoError::Request(_)
            | ModelInfoError::Map(_)
            | ModelInfoError::Catalog(_)
            | ModelInfoError::Generalizations(_)
            | ModelInfoError::Proxy { .. }
            | ModelInfoError::Client { .. }
            | ModelInfoError::Settings { .. }
            | ModelInfoError::Timeout { .. }
            | ModelInfoError::Result(_) => PyValueError::new_err(message),
        }
    }
}
