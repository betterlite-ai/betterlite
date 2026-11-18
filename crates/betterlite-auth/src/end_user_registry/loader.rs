use std::collections::BTreeSet;
use std::fmt::Display;
use std::future::Future;

use betterlite_models::decisions::Json;
use tokio::sync::Mutex;

use super::REGISTRY_ERROR_TTL;
use super::cache::{CachedRegistry, RegistryCache, cache_registry_answer, cached_registry};
use super::errors::RegistryError;

// one bounded registry: where its answer lives, what marks it unusable, and how many ids fit
#[derive(Clone, Copy)]
pub struct RegistrySpec<'key> {
    pub cache_key: &'key str,
    pub overflow_sentinel: &'key str,
    pub max_size: usize,
}

// LiteLLM's _load_bounded_registry: loads are single-flighted so a ttl expiry does not stampede the database
pub async fn load_bounded_registry<Cache, Fetch, Fut, Failure>(
    cache: &Cache,
    spec: RegistrySpec<'_>,
    load_lock: &Mutex<()>,
    fetch_ids: Fetch,
) -> Result<Option<BTreeSet<String>>, RegistryError>
where
    Cache: RegistryCache + ?Sized,
    Fetch: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<String>, Failure>> + Send,
    Failure: Display + Send,
{
    let mut cached = cached_registry(cache, spec).await?;
    if matches!(cached, CachedRegistry::Missing) {
        // the probe guard is released at the semicolon, before the await below
        let waited_for_another_load = load_lock.try_lock().is_err();
        let _load_guard = load_lock.lock().await;
        if waited_for_another_load {
            cached = cached_registry(cache, spec).await?;
        }
    }
    match cached {
        CachedRegistry::Missing => fetch_and_cache_registry(cache, spec, fetch_ids).await,
        CachedRegistry::Unusable => Ok(None),
        CachedRegistry::Known(registry) => Ok(Some(registry)),
    }
}

// LiteLLM's _fetch_and_cache_registry: a failed load fails open for 30 seconds, an oversized one is silent
async fn fetch_and_cache_registry<Cache, Fetch, Fut, Failure>(
    cache: &Cache,
    spec: RegistrySpec<'_>,
    fetch_ids: Fetch,
) -> Result<Option<BTreeSet<String>>, RegistryError>
where
    Cache: RegistryCache + ?Sized,
    Fetch: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<String>, Failure>> + Send,
    Failure: Display + Send,
{
    let registry_ids = match fetch_ids().await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(
                cache_key = spec.cache_key,
                seconds = REGISTRY_ERROR_TTL,
                %error,
                "the registry could not be loaded from the database, so per-id lookups will run and the registry query is suppressed"
            );
            let sentinel = Json::Text(spec.overflow_sentinel.to_owned());
            cache_registry_answer(cache, spec.cache_key, &sentinel, REGISTRY_ERROR_TTL).await;
            return Ok(None);
        }
    };
    let ttl = cache.management_ttl();
    if registry_ids.len() > spec.max_size {
        let sentinel = Json::Text(spec.overflow_sentinel.to_owned());
        cache_registry_answer(cache, spec.cache_key, &sentinel, ttl).await;
        return Ok(None);
    }
    let registry = registry_ids.iter().cloned().collect();
    let answer = Json::List(registry_ids.into_iter().map(Json::Text).collect());
    cache_registry_answer(cache, spec.cache_key, &answer, ttl).await;
    Ok(Some(registry))
}
