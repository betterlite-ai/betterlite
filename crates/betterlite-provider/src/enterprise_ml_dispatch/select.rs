use betterlite_models::identity::provider_id::ProviderId;

// The provider each embedding arm runs, as LiteLLM's get_provider_embedding_config picks it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfig {
    Sap,
    Watsonx,
    Sagemaker,
}

#[must_use]
pub fn embedding_config(provider: ProviderId) -> Option<EmbeddingConfig> {
    if provider == ProviderId::SapGenerativeAiHub {
        return Some(EmbeddingConfig::Sap);
    }
    if provider == ProviderId::Watsonx {
        return Some(EmbeddingConfig::Watsonx);
    }
    if provider == ProviderId::Sagemaker {
        return Some(EmbeddingConfig::Sagemaker);
    }
    None
}

// The provider the rerank branch runs, as LiteLLM's get_provider_rerank_config picks it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankConfig {
    Watsonx,
}

#[must_use]
pub fn rerank_config(provider: ProviderId) -> Option<RerankConfig> {
    if provider == ProviderId::Watsonx {
        return Some(RerankConfig::Watsonx);
    }
    None
}
