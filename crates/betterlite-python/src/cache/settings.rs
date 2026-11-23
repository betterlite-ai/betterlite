use std::path::PathBuf;

use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::configuration::settings::CacheSettings;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::call_type::{CachingSupportedCallType, SUPPORTED_CALL_TYPES};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use serde::Deserialize;

mod tests;

// The settings text a NativeCache is built from: LiteLLM's constructor arguments, absent ones taking its defaults.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CacheSettingsWire {
    #[serde(rename = "type")]
    pub(crate) cache_type: LiteLlmCacheType,
    pub(crate) mode: CacheMode,
    pub(crate) namespace: Option<String>,
    pub(crate) ttl: Option<f64>,
    pub(crate) default_in_memory_ttl: Option<f64>,
    pub(crate) default_in_redis_ttl: Option<f64>,
    pub(crate) supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub(crate) semantic_cache_scope: SemanticCacheScope,
    // litellm.enable_caching_on_provider_specific_optional_params, a module-level global rather than a Cache argument
    pub(crate) include_provider_params: bool,
    pub(crate) redis: RedisOverrides,
    pub(crate) redis_flush_size: Option<usize>,
    pub(crate) disk_cache_dir: Option<PathBuf>,
}

impl Default for CacheSettingsWire {
    fn default() -> Self {
        Self {
            cache_type: LiteLlmCacheType::Local,
            mode: CacheMode::DefaultOn,
            namespace: None,
            ttl: None,
            default_in_memory_ttl: None,
            default_in_redis_ttl: None,
            supported_call_types: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
            redis: RedisOverrides::default(),
            redis_flush_size: None,
            disk_cache_dir: None,
        }
    }
}

impl CacheSettingsWire {
    // LiteLLM's DEFAULT_CACHING_SUPPORTED_CALL_TYPES is the whole list, so an absent field means every call type.
    pub(crate) fn settings(&self) -> CacheSettings {
        CacheSettings {
            cache_type: self.cache_type,
            mode: self.mode,
            namespace: self.namespace.clone(),
            ttl: self.ttl,
            default_in_memory_ttl: self.default_in_memory_ttl,
            default_in_redis_ttl: self.default_in_redis_ttl,
            supported_call_types: self
                .supported_call_types
                .clone()
                .unwrap_or_else(|| SUPPORTED_CALL_TYPES.to_vec()),
            semantic_cache_scope: self.semantic_cache_scope,
            redis: self.redis.clone(),
            redis_flush_size: self.redis_flush_size,
            disk_cache_dir: self.disk_cache_dir.clone(),
            ..CacheSettings::default()
        }
    }

    pub(crate) fn key_settings(&self) -> CacheKeySettings {
        CacheKeySettings {
            cache_type: self.cache_type,
            mode: self.mode,
            namespace: self.namespace.clone(),
            semantic_cache_scope: self.semantic_cache_scope,
            include_provider_params: self.include_provider_params,
        }
    }

    // Cache::open's ttl rule, for the free functions that answer without a backend: a per-kind default wins over ttl.
    pub(crate) fn effective_ttl(&self) -> Option<f64> {
        match self.cache_type {
            LiteLlmCacheType::Local => self.default_in_memory_ttl.or(self.ttl),
            LiteLlmCacheType::Redis
            | LiteLlmCacheType::RedisSemantic
            | LiteLlmCacheType::ValkeySemantic => self.default_in_redis_ttl.or(self.ttl),
            LiteLlmCacheType::S3
            | LiteLlmCacheType::Disk
            | LiteLlmCacheType::QdrantSemantic
            | LiteLlmCacheType::AzureBlob
            | LiteLlmCacheType::Gcs => self.ttl,
        }
    }
}
