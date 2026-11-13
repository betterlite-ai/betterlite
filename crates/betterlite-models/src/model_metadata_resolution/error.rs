use crate::model_catalog_keys::ModelCatalogKeysError;
use crate::model_metadata_projection::ProjectionError;

pub const ADD_MODEL_URL: &str =
    "https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json";

#[must_use]
pub fn model_not_mapped_message(model: &str, provider: Option<&str>) -> String {
    format!(
        "This model isn't mapped yet. model={model}, custom_llm_provider={}. Add it here - {ADD_MODEL_URL}.",
        provider.unwrap_or("None")
    )
}

// Everything the non-dynamic lookup can fail with; LiteLLM's blanket except turns each into one generic Exception.
#[derive(Debug, thiserror::Error)]
pub enum StaticFailure {
    #[error("model metadata: {0}")]
    Projection(#[from] ProjectionError),
    #[error("model cost entry: {0}")]
    Catalog(#[from] ModelCatalogKeysError),
    #[error("model cost entry JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ModelMetadataError {
    #[error("{}", model_not_mapped_message(model, provider.as_deref()))]
    NotMapped {
        model: String,
        provider: Option<String>,
    },
    #[error("{}", model_not_mapped_message(model, provider.as_deref()))]
    Failed {
        model: String,
        provider: Option<String>,
        #[source]
        source: StaticFailure,
    },
}
