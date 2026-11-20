use betterlite_models::resolution::ProviderIdentity;

use crate::embedding_rerank_providers::{RerankConfig, rerank_config};

// LiteLLM's ProviderConfigManager.get_provider_rerank_config, every arm it selects a config class for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedConfig {
    Cohere,
    CohereV2,
    Infinity,
    JinaAi,
    Voyage,
    AzureAi,
    HostedVllm,
    HuggingFace,
    Deepinfra,
    NvidiaRetrieval,
    NvidiaRanking,
    VertexAi,
    FireworksAi,
    Watsonx,
    Dashscope,
    Qwencloud,
    QwenAiPlatform,
}

// The selector chooses by provider, by the call's api_base and by the version parameters; the model decides Nvidia's shape
#[must_use]
pub fn selected_config(
    identity: &ProviderIdentity,
    model: &str,
    api_base: Option<&str>,
    present_version_params: &[String],
) -> SelectedConfig {
    if let ProviderIdentity::Known(provider) = identity {
        if let Some(config) = rerank_config(*provider, api_base, present_version_params) {
            return match config {
                RerankConfig::Cohere => SelectedConfig::Cohere,
                RerankConfig::CohereV2 => SelectedConfig::CohereV2,
                RerankConfig::Infinity => SelectedConfig::Infinity,
                RerankConfig::JinaAi => SelectedConfig::JinaAi,
                RerankConfig::Voyage => SelectedConfig::Voyage,
            };
        }
    }
    provider_config(identity.slug(), model)
}

// The remaining arms of get_provider_rerank_config, keyed by the provider slug it switches on
fn provider_config(provider: &str, model: &str) -> SelectedConfig {
    match provider {
        "azure_ai" => SelectedConfig::AzureAi,
        "hosted_vllm" => SelectedConfig::HostedVllm,
        "huggingface" => SelectedConfig::HuggingFace,
        "deepinfra" => SelectedConfig::Deepinfra,
        "nvidia_nim" => {
            // get_nvidia_nim_rerank_config picks the ranking client for a model under the ranking prefix
            if model.starts_with("ranking/") {
                SelectedConfig::NvidiaRanking
            } else {
                SelectedConfig::NvidiaRetrieval
            }
        }
        "vertex_ai" => SelectedConfig::VertexAi,
        "fireworks_ai" => SelectedConfig::FireworksAi,
        "watsonx" => SelectedConfig::Watsonx,
        "dashscope" => SelectedConfig::Dashscope,
        "qwencloud" => SelectedConfig::Qwencloud,
        "qwen_ai_platform" => SelectedConfig::QwenAiPlatform,
        // LiteLLM's selector ends with `return litellm.CohereRerankConfig()`
        _ => SelectedConfig::Cohere,
    }
}
