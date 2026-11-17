use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::voyage_embedding::VoyageEmbeddingVariant;

// LiteLLM's ProviderConfigManager.get_provider_embedding_config, this family's arms only
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfig {
    Cohere,
    Voyage,
    VoyageContextual,
    VoyageMultimodal,
    JinaAi,
    Infinity,
}

impl EmbeddingConfig {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Cohere => "CohereEmbeddingConfig",
            Self::Voyage => "VoyageEmbeddingConfig",
            Self::VoyageContextual => "VoyageContextualEmbeddingConfig",
            Self::VoyageMultimodal => "VoyageMultimodalEmbeddingConfig",
            Self::JinaAi => "JinaAIEmbeddingConfig",
            Self::Infinity => "InfinityEmbeddingConfig",
        }
    }
}

// LiteLLM's ProviderConfigManager.get_provider_rerank_config, this family's arms only
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankConfig {
    Cohere,
    CohereV2,
    Infinity,
    JinaAi,
    Voyage,
}

impl RerankConfig {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Cohere => "CohereRerankConfig",
            Self::CohereV2 => "CohereRerankV2Config",
            Self::Infinity => "InfinityRerankConfig",
            Self::JinaAi => "JinaAIRerankConfig",
            Self::Voyage => "VoyageRerankConfig",
        }
    }
}

// The Voyage arms test the model's lower-cased text, and the context arm comes first
#[must_use]
pub fn embedding_config(provider: ProviderId, model: &str) -> Option<EmbeddingConfig> {
    match provider {
        ProviderId::Cohere | ProviderId::CohereChat => Some(EmbeddingConfig::Cohere),
        ProviderId::Voyage => Some(match VoyageEmbeddingVariant::for_model(model) {
            VoyageEmbeddingVariant::Contextual => EmbeddingConfig::VoyageContextual,
            VoyageEmbeddingVariant::Multimodal => EmbeddingConfig::VoyageMultimodal,
            VoyageEmbeddingVariant::Text => EmbeddingConfig::Voyage,
        }),
        ProviderId::JinaAi => Some(EmbeddingConfig::JinaAi),
        ProviderId::Infinity => Some(EmbeddingConfig::Infinity),
        _ => None,
    }
}

#[must_use]
pub fn rerank_config(
    provider: ProviderId,
    api_base: Option<&str>,
    present_version_params: &[String],
) -> Option<RerankConfig> {
    match provider {
        ProviderId::Cohere | ProviderId::CohereChat => Some(
            if should_use_cohere_v1_client(api_base, present_version_params) {
                RerankConfig::Cohere
            } else {
                RerankConfig::CohereV2
            },
        ),
        ProviderId::Infinity => Some(RerankConfig::Infinity),
        ProviderId::JinaAi => Some(RerankConfig::JinaAi),
        ProviderId::Voyage => Some(RerankConfig::Voyage),
        _ => None,
    }
}

// litellm/utils.py should_use_cohere_v1_client: an absent or empty base always takes the v2 client,
// and the legacy limit parameter picks v1 only when the base's exact suffix does not say v2
#[must_use]
pub fn should_use_cohere_v1_client(
    api_base: Option<&str>,
    present_version_params: &[String],
) -> bool {
    let Some(base) = api_base.filter(|base| !base.is_empty()) else {
        return false;
    };
    let remains = |name: &str| present_version_params.iter().any(|param| param == name);
    let uses_v1_params = remains("max_chunks_per_doc") && !remains("max_tokens_per_doc");
    base.ends_with("/v1/rerank") || (uses_v1_params && !base.ends_with("/v2/rerank"))
}

mod tests;
