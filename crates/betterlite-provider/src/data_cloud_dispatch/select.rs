use betterlite_models::identity::provider_id::ProviderId;

// The embedding config each provider's arm calls, as LiteLLM's get_provider_embedding_config picks it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfig {
    Databricks,
    Snowflake,
    Oci,
}

#[must_use]
pub fn embedding_config(provider: ProviderId) -> Option<EmbeddingConfig> {
    if provider == ProviderId::Databricks {
        return Some(EmbeddingConfig::Databricks);
    }
    if provider == ProviderId::Snowflake {
        return Some(EmbeddingConfig::Snowflake);
    }
    if provider == ProviderId::Oci {
        return Some(EmbeddingConfig::Oci);
    }
    None
}
