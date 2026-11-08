use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_API_VERSION: &str = "2024-03-13";

pub type WatsonxParams = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatsonxApiParams {
    pub project_id: Option<String>,
    pub space_id: Option<String>,
    pub region_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WatsonxCredentials {
    pub api_key: SecretString,
    pub api_base: String,
    pub token: Option<SecretString>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatsonxAudioRequestFields {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_granularities: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatsonxEndpoint {
    TextGeneration,
    TextGenerationStream,
    Chat,
    ChatStream,
    DeploymentTextGeneration,
    DeploymentTextGenerationStream,
    DeploymentChat,
    DeploymentChatStream,
    Embeddings,
    Prompts,
    AvailableModels,
    Rerank,
}

impl WatsonxEndpoint {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::TextGeneration => "/ml/v1/text/generation",
            Self::TextGenerationStream => "/ml/v1/text/generation_stream",
            Self::Chat => "/ml/v1/text/chat",
            Self::ChatStream => "/ml/v1/text/chat_stream",
            Self::DeploymentTextGeneration => "/ml/v1/deployments/{deployment_id}/text/generation",
            Self::DeploymentTextGenerationStream => {
                "/ml/v1/deployments/{deployment_id}/text/generation_stream"
            }
            Self::DeploymentChat => "/ml/v1/deployments/{deployment_id}/text/chat",
            Self::DeploymentChatStream => "/ml/v1/deployments/{deployment_id}/text/chat_stream",
            Self::Embeddings => "/ml/v1/text/embeddings",
            Self::Prompts => "/ml/v1/prompts",
            Self::AvailableModels => "/ml/v1/foundation_model_specs",
            Self::Rerank => "/ml/v1/text/rerank",
        }
    }
}
