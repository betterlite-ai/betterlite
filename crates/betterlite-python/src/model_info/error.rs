use betterlite_models::catalog::CatalogError;
use betterlite_models::error::ModelsError;
use betterlite_models::model_metadata_resolution::error::{ADD_MODEL_URL, ModelMetadataError};
use betterlite_models::model_metadata_resolution::names::ModelCostMapError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
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
    #[error("{0}")]
    Value(String),
    #[error("model cost map: {0}")]
    Map(#[from] ModelCostMapError),
    #[error("model catalog: {0}")]
    Catalog(#[from] CatalogError),
    #[error("fallback generalizations: {0}")]
    Generalizations(#[from] ModelsError),
    #[error("HTTP client: {0}")]
    Client(String),
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
            | ModelInfoError::Value(_)
            | ModelInfoError::Map(_)
            | ModelInfoError::Catalog(_)
            | ModelInfoError::Generalizations(_)
            | ModelInfoError::Client(_)
            | ModelInfoError::Result(_) => PyValueError::new_err(message),
        }
    }
}
