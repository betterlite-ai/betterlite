use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::voyage_embedding::VoyageEmbeddingVariant;

// LiteLLM's ProviderConfigManager.get_provider_embedding_config, this family's arms only
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum EmbeddingConfig {
    #[strum(serialize = "CohereEmbeddingConfig")]
    Cohere,
    #[strum(serialize = "VoyageEmbeddingConfig")]
    Voyage,
    #[strum(serialize = "VoyageContextualEmbeddingConfig")]
    VoyageContextual,
    #[strum(serialize = "VoyageMultimodalEmbeddingConfig")]
    VoyageMultimodal,
    #[strum(serialize = "JinaAIEmbeddingConfig")]
    JinaAi,
    #[strum(serialize = "InfinityEmbeddingConfig")]
    Infinity,
}

// LiteLLM's ProviderConfigManager.get_provider_rerank_config, this family's arms only
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum RerankConfig {
    #[strum(serialize = "CohereRerankConfig")]
    Cohere,
    #[strum(serialize = "CohereRerankV2Config")]
    CohereV2,
    #[strum(serialize = "InfinityRerankConfig")]
    Infinity,
    #[strum(serialize = "JinaAIRerankConfig")]
    JinaAi,
    #[strum(serialize = "VoyageRerankConfig")]
    Voyage,
}

// The Voyage arms test the model's lower-cased text, and the context arm comes first
#[must_use]
pub fn embedding_config(provider: ProviderId, model: &str) -> Option<EmbeddingConfig> {
    if provider == ProviderId::Voyage {
        return Some(match VoyageEmbeddingVariant::for_model(model) {
            VoyageEmbeddingVariant::Contextual => EmbeddingConfig::VoyageContextual,
            VoyageEmbeddingVariant::Multimodal => EmbeddingConfig::VoyageMultimodal,
            VoyageEmbeddingVariant::Text => EmbeddingConfig::Voyage,
        });
    }
    if matches!(provider, ProviderId::Cohere | ProviderId::CohereChat) {
        return Some(EmbeddingConfig::Cohere);
    }
    if provider == ProviderId::JinaAi {
        return Some(EmbeddingConfig::JinaAi);
    }
    (provider == ProviderId::Infinity).then_some(EmbeddingConfig::Infinity)
}

#[must_use]
pub fn rerank_config(
    provider: ProviderId,
    api_base: Option<&str>,
    present_version_params: &[String],
) -> Option<RerankConfig> {
    if matches!(provider, ProviderId::Cohere | ProviderId::CohereChat) {
        return Some(
            if should_use_cohere_v1_client(api_base, present_version_params) {
                RerankConfig::Cohere
            } else {
                RerankConfig::CohereV2
            },
        );
    }
    if provider == ProviderId::Infinity {
        return Some(RerankConfig::Infinity);
    }
    if provider == ProviderId::JinaAi {
        return Some(RerankConfig::JinaAi);
    }
    (provider == ProviderId::Voyage).then_some(RerankConfig::Voyage)
}

// an absent or empty base takes v2; a legacy limit picks v1 unless the base says v2
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
