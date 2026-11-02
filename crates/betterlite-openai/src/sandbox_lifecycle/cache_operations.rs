use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use super::resolution::{SandboxToolParams, SandboxToolRegistry};

pub const CACHE_TTL_SECONDS: u64 = 15 * 60;
pub const SESSION_SCOPED_IDENTITY_CAP: usize = 10;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct CacheEntry {
    pub container: Value,
    pub params: Option<SandboxToolParams>,
    pub last_accessed: Instant,
    pub identity: Option<String>,
}

#[derive(Default)]
pub struct SandboxCache {
    pub(crate) entries: HashMap<String, CacheEntry>,
}

impl SandboxCache {
    pub fn get(
        &mut self,
        cache_key: &str,
        now: Instant,
    ) -> Option<(Value, Option<SandboxToolParams>)> {
        let entry = self.entries.get_mut(cache_key)?;
        entry.last_accessed = now;
        Some((entry.container.clone(), entry.params.clone()))
    }

    fn identity_entries(&self, identity: &str) -> Vec<(String, Instant)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.identity.as_deref() == Some(identity))
            .map(|(key, entry)| (key.clone(), entry.last_accessed))
            .collect()
    }

    fn expired_keys(&self, now: Instant, ttl: Duration) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| now.saturating_duration_since(entry.last_accessed) > ttl)
            .map(|(key, _)| key.clone())
            .collect()
    }
}

pub async fn get_or_create_container<C, R, T, Clk>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    cache_key: Option<&str>,
    identity: Option<&str>,
    clock: &Clk,
) -> Result<(Value, Option<SandboxToolParams>), super::lifecycle::LifecycleError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
    Clk: Clock,
{
    let now = clock.now();
    if let Some(key) = cache_key
        && let Some(cached) = cache.get(key, now)
    {
        return Ok(cached);
    }
    let (container, params) = provider.create_container().await?;
    if let Some(key) = cache_key {
        if let Some(scope) = identity {
            evict_lru_session_if_over_cap(provider, cache, scope).await;
        }
        cache.entries.insert(
            key.to_owned(),
            CacheEntry {
                container: container.clone(),
                params: params.clone(),
                last_accessed: now,
                identity: identity.map(str::to_owned),
            },
        );
    }
    Ok((container, params))
}

pub async fn evict_lru_session_if_over_cap<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    identity: &str,
) where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    let entries = cache.identity_entries(identity);
    if entries.len() < SESSION_SCOPED_IDENTITY_CAP {
        return;
    }
    let Some((lru_key, _)) = entries
        .into_iter()
        .min_by_key(|(_, last_accessed)| *last_accessed)
    else {
        return;
    };
    let Some(entry) = cache.entries.remove(&lru_key) else {
        return;
    };
    provider
        .delete_container(entry.params.as_ref(), &entry.container)
        .await;
}

pub async fn prune_expired_cache<C, R, T, Clk>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    clock: &Clk,
) where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
    Clk: Clock,
{
    let now = clock.now();
    let ttl = Duration::from_secs(CACHE_TTL_SECONDS);
    for key in cache.expired_keys(now, ttl) {
        let Some(entry) = cache.entries.remove(&key) else {
            continue;
        };
        provider
            .delete_container(entry.params.as_ref(), &entry.container)
            .await;
    }
}

include!("cache_operations/tests/cases.rs");
