use std::convert::Infallible;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;

use crate::google_token_counter::counter::{StudioTokenCounter, VertexTokenCounter};
use crate::google_token_counter::error::ModelInfoError;

#[derive(Debug, Clone)]
pub struct VertexModelInfo {
    pub settings: VertexSettings,
}

impl VertexModelInfo {
    // LiteLLM's VertexAIModelInfo and GeminiModelInfo factories pick one counter class each and read no model
    #[must_use]
    pub fn get_token_counter(&self) -> VertexTokenCounter {
        VertexTokenCounter {
            settings: self.settings.clone(),
        }
    }

    // LiteLLM's VertexAIModelInfo raises NotImplementedError from all five of these operations
    pub fn validate_environment(&self) -> Result<Infallible, ModelInfoError> {
        tracing::debug!(
            project = ?self.settings.project,
            "Vertex AI models do not support validate_environment"
        );
        Err(ModelInfoError::NotSupported)
    }

    pub fn get_models(&self) -> Result<Infallible, ModelInfoError> {
        tracing::debug!(
            project = ?self.settings.project,
            "Vertex AI models do not support get_models"
        );
        Err(ModelInfoError::NotSupported)
    }

    pub fn get_api_key() -> Result<Infallible, ModelInfoError> {
        tracing::debug!("Vertex AI models do not support get_api_key");
        Err(ModelInfoError::NotSupported)
    }

    pub fn get_api_base() -> Result<Infallible, ModelInfoError> {
        tracing::debug!("Vertex AI models do not support get_api_base");
        Err(ModelInfoError::NotSupported)
    }

    pub fn get_base_model(model: &str) -> Result<Infallible, ModelInfoError> {
        tracing::debug!(model, "Vertex AI models do not support get_base_model");
        Err(ModelInfoError::NotSupported)
    }
}

#[derive(Debug, Clone)]
pub struct GeminiModelInfo {
    pub settings: Settings,
}

impl GeminiModelInfo {
    #[must_use]
    pub fn get_token_counter(&self) -> StudioTokenCounter {
        StudioTokenCounter {
            settings: self.settings.clone(),
        }
    }
}
