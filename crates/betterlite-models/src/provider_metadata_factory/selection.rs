use crate::fireworks_catalog_metadata::{self, Catalog};
use crate::identity::provider_id::ProviderId;

use super::adapters::{
    AzureFoundryModelInfo, DynamicModelInfo, FireworksAIConfig, LemonadeChatConfig,
    ProviderSpecificModelInfo,
};

#[derive(Debug)]
pub enum ProviderModelMetadata {
    FireworksAi(FireworksAIConfig),
    OpenAi,
    Gemini,
    VertexAi,
    LitellmProxy,
    Topaz,
    Anthropic,
    Xai,
    Ollama,
    Vllm,
    Lemonade(LemonadeChatConfig),
    Clarifai,
    EdenAi,
    Bedrock,
    AzureAi(AzureFoundryModelInfo),
}

type AdapterFactory = fn(Option<&str>) -> ProviderModelMetadata;

// The ProviderConfigManager table is exact provider equality; no prefix or substring resolves an adapter.
const ADAPTER_ROWS: &[(ProviderId, AdapterFactory)] = &[
    (ProviderId::FireworksAi, |_| {
        ProviderModelMetadata::FireworksAi(FireworksAIConfig::default())
    }),
    (ProviderId::Openai, |_| ProviderModelMetadata::OpenAi),
    (ProviderId::Gemini, |_| ProviderModelMetadata::Gemini),
    (ProviderId::VertexAi, |_| ProviderModelMetadata::VertexAi),
    (ProviderId::LitellmProxy, |_| {
        ProviderModelMetadata::LitellmProxy
    }),
    (ProviderId::Topaz, |_| ProviderModelMetadata::Topaz),
    (ProviderId::Anthropic, |_| ProviderModelMetadata::Anthropic),
    (ProviderId::Xai, |_| ProviderModelMetadata::Xai),
    (ProviderId::Ollama, |_| ProviderModelMetadata::Ollama),
    (ProviderId::OllamaChat, |_| ProviderModelMetadata::Ollama),
    (ProviderId::Vllm, |_| ProviderModelMetadata::Vllm),
    (ProviderId::HostedVllm, |_| ProviderModelMetadata::Vllm),
    (ProviderId::Lemonade, |_| {
        ProviderModelMetadata::Lemonade(LemonadeChatConfig::default())
    }),
    (ProviderId::Clarifai, |_| ProviderModelMetadata::Clarifai),
    (ProviderId::Edenai, |_| ProviderModelMetadata::EdenAi),
    (ProviderId::Bedrock, |_| ProviderModelMetadata::Bedrock),
    (ProviderId::AzureAi, |model| {
        ProviderModelMetadata::AzureAi(AzureFoundryModelInfo {
            model: model.map(ToOwned::to_owned),
        })
    }),
];

impl ProviderModelMetadata {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::FireworksAi(_) => "FireworksAIConfig",
            Self::OpenAi => "OpenAIGPTConfig",
            Self::Gemini => "GeminiModelInfo",
            Self::VertexAi => "VertexAIModelInfo",
            Self::LitellmProxy => "LiteLLMProxyChatConfig",
            Self::Topaz => "TopazModelInfo",
            Self::Anthropic => "AnthropicModelInfo",
            Self::Xai => "XAIModelInfo",
            Self::Ollama => "OllamaModelInfo",
            Self::Vllm => "VLLMModelInfo",
            Self::Lemonade(_) => "LemonadeChatConfig",
            Self::Clarifai => "ClarifaiConfig",
            Self::EdenAi => "EdenAIChatConfig",
            Self::Bedrock => "BedrockModelInfo",
            Self::AzureAi(_) => "AzureFoundryModelInfo",
        }
    }

    #[must_use]
    pub const fn dynamic_model_info(&self) -> Option<DynamicModelInfo> {
        match self {
            Self::Ollama => Some(DynamicModelInfo::Ollama),
            Self::Lemonade(_) => Some(DynamicModelInfo::Lemonade),
            Self::FireworksAi(_)
            | Self::OpenAi
            | Self::Gemini
            | Self::VertexAi
            | Self::LitellmProxy
            | Self::Topaz
            | Self::Anthropic
            | Self::Xai
            | Self::Vllm
            | Self::Clarifai
            | Self::EdenAi
            | Self::Bedrock
            | Self::AzureAi(_) => None,
        }
    }

    pub fn get_provider_info(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        model: &str,
    ) -> Option<ProviderSpecificModelInfo> {
        match self {
            Self::FireworksAi(config) => Some(config.get_provider_info(catalog, generation, model)),
            Self::Xai => Some(ProviderSpecificModelInfo {
                supports_function_calling: None,
                supports_prompt_caching: None,
                supports_reasoning: None,
                supports_vision: None,
                supports_pdf_input: None,
                supports_web_search: Some(true),
            }),
            Self::OpenAi
            | Self::Gemini
            | Self::VertexAi
            | Self::LitellmProxy
            | Self::Topaz
            | Self::Anthropic
            | Self::Ollama
            | Self::Vllm
            | Self::Lemonade(_)
            | Self::Clarifai
            | Self::EdenAi
            | Self::Bedrock
            | Self::AzureAi(_) => None,
        }
    }

    #[must_use]
    pub fn stored_model(&self) -> Option<&str> {
        match self {
            Self::AzureAi(config) => config.model.as_deref(),
            Self::FireworksAi(_)
            | Self::OpenAi
            | Self::Gemini
            | Self::VertexAi
            | Self::LitellmProxy
            | Self::Topaz
            | Self::Anthropic
            | Self::Xai
            | Self::Ollama
            | Self::Vllm
            | Self::Lemonade(_)
            | Self::Clarifai
            | Self::EdenAi
            | Self::Bedrock => None,
        }
    }

    #[must_use]
    pub fn get_model_cost_key(&self, model: &str) -> Option<String> {
        match self {
            Self::FireworksAi(_) => Some(fireworks_catalog_metadata::get_model_cost_key(model)),
            Self::OpenAi
            | Self::Gemini
            | Self::VertexAi
            | Self::LitellmProxy
            | Self::Topaz
            | Self::Anthropic
            | Self::Xai
            | Self::Ollama
            | Self::Vllm
            | Self::Lemonade(_)
            | Self::Clarifai
            | Self::EdenAi
            | Self::Bedrock
            | Self::AzureAi(_) => None,
        }
    }
}

#[must_use]
pub fn select(model: Option<&str>, provider: ProviderId) -> Option<ProviderModelMetadata> {
    ADAPTER_ROWS
        .iter()
        .find(|(id, _)| *id == provider)
        .map(|(_, adapter)| adapter(model))
}

#[must_use]
pub fn get_provider_info(
    model: &str,
    custom_llm_provider: Option<&str>,
    catalog: &Catalog,
    generation: u64,
) -> Option<ProviderSpecificModelInfo> {
    let provider = custom_llm_provider?.parse::<ProviderId>().ok()?;
    let mut adapter = select(Some(model), provider)?;
    adapter.get_provider_info(catalog, generation, model)
}
