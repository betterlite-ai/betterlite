use thiserror::Error;

use crate::identity::provider_id::ProviderId;

#[derive(Debug, Error)]
pub enum ProviderDefaultsError {
    #[error("{provider} has no OpenAI-compatible default entry in provider_defaults_b")]
    UnknownProvider { provider: ProviderId },
    #[error("ragflow model {model} must have the form ragflow/{{chat|agent}}/{{id}}/{{model}}")]
    InvalidRagflowModel { model: String },
}
