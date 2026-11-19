use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

use super::errors::RegistryError;
use super::loader::RegistrySpec;

// the one cache surface a bounded registry needs, so a read or write failure is a named state
#[async_trait]
pub trait RegistryCache: Send + Sync {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError>;
    async fn write(&self, cache_key: &str, answer: &Json, ttl: f64) -> Result<(), AuthCacheError>;
    fn management_ttl(&self) -> f64;
}

#[async_trait]
impl<T: AuthCacheTier + 'static> RegistryCache for AuthCache<T> {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(cache_key, false).await)
    }

    async fn write(&self, cache_key: &str, answer: &Json, ttl: f64) -> Result<(), AuthCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(cache_key), answer, options).await
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}

// the three states a cache read can produce: no answer yet, an unusable registry, or a usable id set
pub enum CachedRegistry {
    Missing,
    Unusable,
    Known(BTreeSet<String>),
}

// LiteLLM's _cached_registry: the sentinel means unusable, a list is a string set, anything else is uncached
pub async fn cached_registry<Cache>(
    cache: &Cache,
    spec: RegistrySpec<'_>,
) -> Result<CachedRegistry, RegistryError>
where
    Cache: RegistryCache + ?Sized,
{
    let Some(cached) = cache
        .read(spec.cache_key)
        .await
        .map_err(RegistryError::CacheRead)?
    else {
        return Ok(CachedRegistry::Missing);
    };
    Ok(match cached {
        Json::Text(text) if text == spec.overflow_sentinel => CachedRegistry::Unusable,
        Json::List(entries) => CachedRegistry::Known(
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    Json::Text(text) => Some(text),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => None,
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            CachedRegistry::Missing
        }
    })
}

// best effort: a cache backend failure never turns a computed registry answer into a failed request
pub async fn cache_registry_answer<Cache>(cache: &Cache, cache_key: &str, answer: &Json, ttl: f64)
where
    Cache: RegistryCache + ?Sized,
{
    if let Err(error) = cache.write(cache_key, answer, ttl).await {
        tracing::warn!(cache_key, %error, "Failed to cache registry");
    }
}
