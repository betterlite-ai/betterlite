use std::path::PathBuf;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup as RedisEnvironmentLookup;
use betterlite_redis::redis_connection_settings::model::scalars::Scalar;
use thiserror::Error;
use tokio::task::JoinSet;

use super::cache::Backend;
use super::settings::{CacheDependencies, CacheSettings, MEMORY_MAX_SIZE_PER_ITEM_KIB};
use super::type_not_supported::CacheTypeNotSupportedError;
use crate::backend::BackendError;
use crate::backend::memory::MemoryCache;
use crate::backend::memory::local::LocalMemoryOps;
use crate::disk::{Clock, DiskCache, DiskError};
use crate::types::cache_type::LiteLlmCacheType;

const REDIS_CLUSTER_OWNER: &str = "B5640";
const REDIS_CLUSTER_NODES: &str = "REDIS_CLUSTER_NODES";

#[derive(Debug, Error)]
pub enum BackendConstructionError {
    #[error(transparent)]
    Unsupported(#[from] CacheTypeNotSupportedError),
    #[error("the in-memory cache could not be created")]
    Memory {
        #[source]
        source: BackendError,
    },
    #[error("the disk cache could not be created")]
    Disk {
        #[source]
        source: DiskError,
    },
    #[error("the redis cache could not be created")]
    Redis {
        #[source]
        source: RedisCacheError,
    },
}

pub(crate) async fn build_backend(
    settings: &CacheSettings,
    environment: &(dyn EnvironmentLookup + Sync),
    dependencies: &CacheDependencies,
) -> Result<Backend, BackendConstructionError> {
    match settings.cache_type {
        LiteLlmCacheType::Local => local_backend(dependencies),
        LiteLlmCacheType::Redis => redis_backend(settings, environment, dependencies).await,
        LiteLlmCacheType::Disk => disk_backend(settings, dependencies).await,
        LiteLlmCacheType::RedisSemantic => Err(unsupported_owner(settings.cache_type, "B4382")),
        LiteLlmCacheType::ValkeySemantic => Err(unsupported_owner(settings.cache_type, "B8296")),
        LiteLlmCacheType::QdrantSemantic => Err(unsupported_owner(settings.cache_type, "B4383")),
        LiteLlmCacheType::S3 => Err(unsupported_owner(settings.cache_type, "B4350")),
        LiteLlmCacheType::AzureBlob => Err(unsupported_owner(settings.cache_type, "B4351")),
        LiteLlmCacheType::Gcs => Err(BackendConstructionError::Unsupported(
            CacheTypeNotSupportedError::NotImplemented {
                cache_type: settings.cache_type,
            },
        )),
    }
}

fn local_backend(dependencies: &CacheDependencies) -> Result<Backend, BackendConstructionError> {
    let operations = LocalMemoryOps::new(
        MEMORY_MAX_SIZE_PER_ITEM_KIB,
        Arc::clone(&dependencies.clock),
    )
    .map_err(|source| BackendConstructionError::Memory { source })?;
    Ok(Backend::Memory(Box::new(MemoryCache { operations })))
}

// opening the sqlite file blocks, so it runs on the blocking pool like disk.rs's async twins
async fn disk_backend(
    settings: &CacheSettings,
    dependencies: &CacheDependencies,
) -> Result<Backend, BackendConstructionError> {
    let directory: Option<PathBuf> = settings.disk_cache_dir.clone();
    let clock: Clock = Arc::clone(&dependencies.clock);
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || DiskCache::open(directory.as_deref(), clock));
    let joined = tasks
        .join_next()
        .await
        .ok_or(DiskError::BlockingMissing)
        .map_err(|source| BackendConstructionError::Disk { source })?;
    let opened = joined.map_err(|source| BackendConstructionError::Disk {
        source: DiskError::Blocking { source },
    })?;
    opened
        .map(|cache| Backend::Disk(Box::new(cache)))
        .map_err(|source| BackendConstructionError::Disk { source })
}

const fn unsupported_owner(
    cache_type: LiteLlmCacheType,
    owner: &'static str,
) -> BackendConstructionError {
    BackendConstructionError::Unsupported(CacheTypeNotSupportedError::OwnedByTask {
        cache_type,
        owner,
    })
}

async fn redis_backend(
    settings: &CacheSettings,
    environment: &(dyn EnvironmentLookup + Sync),
    dependencies: &CacheDependencies,
) -> Result<Backend, BackendConstructionError> {
    // LiteLLM selects cluster mode from the constructor list first, then from the environment
    let cluster_nodes = environment
        .lookup(REDIS_CLUSTER_NODES)
        .is_some_and(|nodes| !nodes.trim().is_empty());
    if settings
        .redis
        .startup_nodes
        .as_ref()
        .is_some_and(Scalar::is_truthy)
        || cluster_nodes
    {
        return Err(BackendConstructionError::Unsupported(
            CacheTypeNotSupportedError::RedisCluster {
                owner: REDIS_CLUSTER_OWNER,
            },
        ));
    }
    let mut overrides = settings.redis.clone();
    // the single-node client is built without a startup node list, as LiteLLM's Cache does
    overrides.startup_nodes = None;
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: settings.namespace.clone(),
        default_ttl: settings.redis_default_ttl,
        redis_flush_size: settings.redis_flush_size,
        default_major_version: settings.redis_default_major_version,
        factory: Arc::clone(&dependencies.redis_factory),
        store: Arc::clone(&dependencies.redis_store),
        hooks: Arc::clone(&dependencies.redis_hooks),
        gate: Arc::clone(&dependencies.redis_gate),
    };
    let redis_environment = RedisEnvironment {
        config: environment,
    };
    let cache = connect(options, &redis_environment)
        .await
        .map_err(|source| BackendConstructionError::Redis { source })?;
    Ok(Backend::Redis(Box::new(cache)))
}

struct RedisEnvironment<'config> {
    config: &'config (dyn EnvironmentLookup + Sync),
}

impl RedisEnvironmentLookup for RedisEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        // an empty env var counts as unset, so a variable the process sets empty never reaches redis
        self.config
            .lookup(name)
            .filter(|value| !value.trim().is_empty())
    }
}
