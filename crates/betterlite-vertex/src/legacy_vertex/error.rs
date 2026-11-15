use betterlite_cache::llm_client_cache::LlmClientCacheError;
use betterlite_core::message_utilities::response_streams::StreamError;
use thiserror::Error;

use crate::gemini_options::error::GeminiOptionsError;
use crate::legacy_prediction::error::{LegacyPredictionError, PredictionError};
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum LegacyVertexError {
    #[error("the legacy vertex client cache could not be read")]
    Cache(#[from] LlmClientCacheError),
    #[error("the legacy vertex credential could not be resolved")]
    Credentials(#[from] CredentialCacheError),
    #[error(transparent)]
    Defaults(#[from] GeminiOptionsError),
    #[error("`safety_settings` must be a list")]
    SafetyNotList,
    #[error("`safety_settings` must be a list of dicts")]
    SafetyItemNotMapping,
    #[error("`model_id` is required for the private endpoint")]
    MissingModelId,
    #[error("location is required for the legacy vertex call")]
    MissingLocation,
    #[error("Vertex project and location are required for custom endpoint")]
    CustomEndpointContext,
    #[error(transparent)]
    Prediction(#[from] PredictionError),
    #[error(transparent)]
    Delegated(#[from] LegacyPredictionError),
    #[error(transparent)]
    Stream(#[from] StreamError),
}

impl LegacyVertexError {
    // LiteLLM turns every synchronous failure into APIConnectionError; a delegated call keeps the Vertex status B1328 maps
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::SafetyNotList | Self::SafetyItemNotMapping => 400,
            Self::Delegated(error) => error.status_code(),
            Self::Cache(_)
            | Self::Credentials(_)
            | Self::Defaults(_)
            | Self::MissingModelId
            | Self::MissingLocation
            | Self::CustomEndpointContext
            | Self::Prediction(_)
            | Self::Stream(_) => 500,
        }
    }
}
