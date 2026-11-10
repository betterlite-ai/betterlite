use crate::fireworks_catalog_metadata::{self, Catalog};
use crate::identity::provider_id::ProviderId;

use super::adapters::{
    AzureFoundryModelInfo, DynamicModelInfo, FireworksAIConfig, LemonadeChatConfig,
    ProviderSpecificModelInfo, StatelessAdapter,
};

#[derive(Clone, Copy, Debug)]
pub struct MetadataQuery<'catalog> {
    pub catalog: &'catalog Catalog,
    pub generation: u64,
}

#[derive(Debug)]
pub enum ProviderModelMetadata {
    FireworksAi(FireworksAIConfig),
    Xai,
    Ollama,
    Lemonade(LemonadeChatConfig),
    AzureAi(AzureFoundryModelInfo),
    Stateless(StatelessAdapter),
}

type AdapterFactory = fn(Option<&str>) -> ProviderModelMetadata;

// The ProviderConfigManager table is exact provider equality; no prefix or substring resolves an adapter.
const ADAPTER_ROWS: &[(ProviderId, AdapterFactory)] = &[
    (ProviderId::FireworksAi, |_| {
        ProviderModelMetadata::FireworksAi(FireworksAIConfig::default())
    }),
    (ProviderId::Openai, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::OpenAi)
    }),
    (ProviderId::Gemini, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Gemini)
    }),
    (ProviderId::VertexAi, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::VertexAi)
    }),
    (ProviderId::LitellmProxy, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::LitellmProxy)
    }),
    (ProviderId::Topaz, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Topaz)
    }),
    (ProviderId::Anthropic, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Anthropic)
    }),
    (ProviderId::Xai, |_| ProviderModelMetadata::Xai),
    (ProviderId::Ollama, |_| ProviderModelMetadata::Ollama),
    (ProviderId::OllamaChat, |_| ProviderModelMetadata::Ollama),
    (ProviderId::Vllm, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Vllm)
    }),
    (ProviderId::HostedVllm, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Vllm)
    }),
    (ProviderId::Lemonade, |_| {
        ProviderModelMetadata::Lemonade(LemonadeChatConfig::default())
    }),
    (ProviderId::Clarifai, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Clarifai)
    }),
    (ProviderId::Edenai, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::EdenAi)
    }),
    (ProviderId::Bedrock, |_| {
        ProviderModelMetadata::Stateless(StatelessAdapter::Bedrock)
    }),
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
            Self::Xai => "XAIModelInfo",
            Self::Ollama => "OllamaModelInfo",
            Self::Lemonade(_) => "LemonadeChatConfig",
            Self::AzureAi(_) => "AzureFoundryModelInfo",
            Self::Stateless(adapter) => adapter.class_name(),
        }
    }

    #[must_use]
    pub const fn dynamic_model_info(&self) -> Option<DynamicModelInfo> {
        match self {
            Self::Ollama => Some(DynamicModelInfo::Ollama),
            Self::Lemonade(_) => Some(DynamicModelInfo::Lemonade),
            Self::FireworksAi(_) | Self::Xai | Self::AzureAi(_) | Self::Stateless(_) => None,
        }
    }

    pub fn get_provider_info(
        &mut self,
        model: &str,
        query: MetadataQuery<'_>,
    ) -> Option<ProviderSpecificModelInfo> {
        match self {
            Self::FireworksAi(config) => {
                Some(config.get_provider_info(query.catalog, query.generation, model))
            }
            Self::Xai => Some(ProviderSpecificModelInfo {
                supports_function_calling: None,
                supports_prompt_caching: None,
                supports_reasoning: None,
                supports_vision: None,
                supports_pdf_input: None,
                supports_web_search: Some(true),
            }),
            Self::Ollama | Self::Lemonade(_) | Self::AzureAi(_) | Self::Stateless(_) => None,
        }
    }

    #[must_use]
    pub fn get_model_cost_key(&self, model: &str) -> Option<String> {
        match self {
            Self::FireworksAi(_) => Some(fireworks_catalog_metadata::get_model_cost_key(model)),
            Self::Xai
            | Self::Ollama
            | Self::Lemonade(_)
            | Self::AzureAi(_)
            | Self::Stateless(_) => None,
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
    query: MetadataQuery<'_>,
) -> Option<ProviderSpecificModelInfo> {
    let provider = custom_llm_provider?.parse::<ProviderId>().ok()?;
    let mut adapter = select(Some(model), provider)?;
    adapter.get_provider_info(model, query)
}
