use std::path::PathBuf;
use std::sync::Arc;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;

use crate::backend::CacheBackend;
use crate::disk::Clock;
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::{CachingSupportedCallType, SUPPORTED_CALL_TYPES};
use crate::types::semantic::SemanticCacheScope;

// litellm.constants.DEFAULT_REDIS_MAJOR_VERSION, which RedisCache uses when INFO is unreadable
pub const DEFAULT_REDIS_MAJOR_VERSION: u32 = 7;
// BaseCache's default ttl, used when litellm.default_redis_ttl is unset
pub const DEFAULT_REDIS_TTL_SECONDS: i64 = 60;
// InMemoryCache's max_size_per_item, counted in KiB
pub(crate) const MEMORY_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

pub struct CacheSettings {
    pub cache_type: LiteLlmCacheType,
    pub mode: CacheMode,
    pub namespace: Option<String>,
    pub ttl: Option<f64>,
    pub default_in_memory_ttl: Option<f64>,
    pub default_in_redis_ttl: Option<f64>,
    pub supported_call_types: Vec<CachingSupportedCallType>,
    pub semantic_cache_scope: SemanticCacheScope,
    pub redis: RedisOverrides,
    pub redis_flush_size: Option<usize>,
    pub redis_default_ttl: i64,
    pub redis_default_major_version: u32,
    pub disk_cache_dir: Option<PathBuf>,
    pub backend: Option<Arc<dyn CacheBackend>>,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            cache_type: LiteLlmCacheType::Local,
            mode: CacheMode::DefaultOn,
            namespace: None,
            ttl: None,
            default_in_memory_ttl: None,
            default_in_redis_ttl: None,
            supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
            semantic_cache_scope: SemanticCacheScope::Key,
            redis: RedisOverrides::default(),
            redis_flush_size: None,
            redis_default_ttl: DEFAULT_REDIS_TTL_SECONDS,
            redis_default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
            disk_cache_dir: None,
            backend: None,
        }
    }
}

pub struct CacheDependencies {
    pub clock: Clock,
    pub redis_factory: Arc<LiveClientFactory>,
    pub redis_store: Arc<RedisClientStore>,
    pub redis_hooks: Arc<dyn ServiceHooks>,
    pub redis_gate: Arc<dyn BreakerGate>,
}
