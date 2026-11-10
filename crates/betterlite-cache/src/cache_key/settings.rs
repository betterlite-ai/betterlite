use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::semantic::SemanticCacheScope;

// The Cache fields key derivation reads, with the provider-param flag LiteLLM keeps as a module-level global instead of config.
#[derive(Clone, Debug)]
pub struct CacheKeySettings {
    pub cache_type: LiteLlmCacheType,
    pub mode: CacheMode,
    pub namespace: Option<String>,
    pub semantic_cache_scope: SemanticCacheScope,
    pub include_provider_params: bool,
}

impl CacheKeySettings {
    #[must_use]
    pub const fn is_semantic_cache(&self) -> bool {
        match self.cache_type {
            LiteLlmCacheType::RedisSemantic
            | LiteLlmCacheType::ValkeySemantic
            | LiteLlmCacheType::QdrantSemantic => true,
            LiteLlmCacheType::Local
            | LiteLlmCacheType::Redis
            | LiteLlmCacheType::S3
            | LiteLlmCacheType::Disk
            | LiteLlmCacheType::AzureBlob
            | LiteLlmCacheType::Gcs => false,
        }
    }
}
