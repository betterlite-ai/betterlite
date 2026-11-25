use std::path::PathBuf;

use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use serde::Deserialize;

// InMemoryCache's constructor arguments that the native memory tier takes.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MemorySettings {
    pub(crate) max_size_in_memory: Option<usize>,
    pub(crate) max_size_per_item: Option<i64>,
    pub(crate) default_ttl: Option<f64>,
}

// DiskCache's constructor argument: LiteLLM's .litellm_cache is used when it is absent.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DiskSettings {
    pub(crate) disk_cache_dir: Option<PathBuf>,
}

// RedisCache's constructor arguments, with every other redis kwarg passing through the overrides.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RedisSettings {
    pub(crate) namespace: Option<String>,
    pub(crate) default_ttl: Option<i64>,
    pub(crate) redis_flush_size: Option<usize>,
    pub(crate) overrides: RedisOverrides,
}

// DualCache's constructor arguments; the batch access window keeps LiteLLM's ten second default.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DualSettings {
    pub(crate) default_in_memory_ttl: Option<f64>,
    pub(crate) default_redis_ttl: Option<f64>,
    pub(crate) batch_cache_expiry: Option<f64>,
    pub(crate) max_redis_batch_cache_size: Option<usize>,
}
