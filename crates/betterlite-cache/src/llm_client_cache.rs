use std::collections::BTreeMap;
use std::num::TryFromIntError;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::dynamic_logging_cache::{Clock, lookup_marked, store_marked};
use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use thiserror::Error;

use super::evicted_client_closer::{EvictableClient, EvictedClientCloser, current_event_loop_id};

pub const DEFAULT_MAX_SIZE_IN_MEMORY: usize = 200;
pub const DEFAULT_TTL_SECONDS: f64 = 600.0;

const ITEM_KIB: i64 = 1024;

#[derive(Debug, Error)]
pub enum LlmClientCacheError {
    #[error("the llm client cache cannot hold {capacity} entries")]
    Capacity {
        capacity: usize,
        #[source]
        source: TryFromIntError,
    },
    #[error("the llm client cache rejected the entry")]
    Cache {
        #[source]
        source: Box<CacheError>,
    },
}

impl From<CacheError> for LlmClientCacheError {
    fn from(source: CacheError) -> Self {
        Self::Cache {
            source: Box::new(source),
        }
    }
}

struct Inner<Client: EvictableClient> {
    markers: LocalTtlCache,
    values: BTreeMap<String, Arc<Client>>,
}

pub struct LlmClientCache<Client: EvictableClient> {
    capacity: usize,
    clock: Clock,
    closer: Arc<EvictedClientCloser<Client>>,
    inner: Mutex<Inner<Client>>,
}

fn event_loop_key(key: &str) -> String {
    let Some(loop_id) = current_event_loop_id() else {
        return key.to_owned();
    };
    format!("{key}-{loop_id}")
}

impl<Client: EvictableClient> LlmClientCache<Client> {
    pub fn new(
        capacity: usize,
        default_ttl: f64,
        clock: Clock,
        closer: Arc<EvictedClientCloser<Client>>,
    ) -> Result<Self, LlmClientCacheError> {
        let capacity_limit = i64::try_from(capacity)
            .map_err(|source| LlmClientCacheError::Capacity { capacity, source })?;
        let markers = LocalTtlCache::with_clock(
            Some(capacity_limit),
            Some(default_ttl),
            None,
            ITEM_KIB,
            Arc::clone(&clock),
        )?;
        Ok(Self {
            capacity,
            clock,
            closer,
            inner: Mutex::new(Inner {
                markers,
                values: BTreeMap::new(),
            }),
        })
    }

    // LiteLLM's evict_cache removes an expired or capacity-victim entry before the overwrite, so its client leaves with it
    fn replaces_an_evicted_entry(&self, markers: &LocalTtlCache, key: &str, now: f64) -> bool {
        let entries = markers.ttl_entries();
        let Some((_marker, key_expiry)) = entries.iter().find(|(marker, _expiry)| marker == key)
        else {
            return false;
        };
        if *key_expiry <= now {
            return true;
        }
        let is_live = |expiry: f64| expiry.total_cmp(&now).is_gt();
        let live_count = entries
            .iter()
            .filter(|(_, expiry)| is_live(*expiry))
            .count();
        if live_count < self.capacity {
            return false;
        }
        let victim = entries
            .iter()
            .filter(|(_, expiry)| is_live(*expiry))
            .map(|(marker, expiry)| (marker, *expiry))
            .min_by(|(left_key, left_expiry), (right_key, right_expiry)| {
                left_expiry
                    .total_cmp(right_expiry)
                    .then_with(|| left_key.cmp(right_key))
            });
        victim.is_some_and(|(victim_key, _expiry)| victim_key.as_str() == key)
    }

    pub fn set_cache(
        &self,
        key: &str,
        client: Arc<Client>,
        ttl: Option<f64>,
        litellm_owned_client: bool,
    ) -> Result<(), LlmClientCacheError> {
        if litellm_owned_client {
            self.closer.mark_owned(&client);
        }
        let decorated = event_loop_key(key);
        let now = (self.clock)();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let replaced_evicted = self.replaces_an_evicted_entry(&inner.markers, &decorated, now);
        let (evicted, replaced) = store_marked(
            &mut inner.markers,
            &mut inner.values,
            decorated,
            client,
            ttl,
        )?;
        drop(guard);
        let mut retired = evicted;
        if replaced_evicted {
            retired.extend(replaced);
        }
        self.retire(&retired);
        Ok(())
    }

    pub async fn async_set_cache(
        &self,
        key: &str,
        client: Arc<Client>,
        ttl: Option<f64>,
        litellm_owned_client: bool,
    ) -> Result<(), LlmClientCacheError> {
        self.set_cache(key, client, ttl, litellm_owned_client)?;
        self.closer.reap_async().await;
        Ok(())
    }

    pub fn get_cache(&self, key: &str) -> Result<Option<Arc<Client>>, LlmClientCacheError> {
        let decorated = event_loop_key(key);
        self.closer.reap();
        self.lookup(&decorated)
    }

    pub async fn async_get_cache(
        &self,
        key: &str,
    ) -> Result<Option<Arc<Client>>, LlmClientCacheError> {
        let decorated = event_loop_key(key);
        self.closer.reap_async().await;
        self.lookup(&decorated)
    }

    fn lookup(&self, decorated: &str) -> Result<Option<Arc<Client>>, LlmClientCacheError> {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let (cached, expired) = lookup_marked(&mut inner.markers, &mut inner.values, decorated)?;
        drop(guard);
        let Some(expired_client) = expired else {
            return Ok(cached);
        };
        self.retire(&[expired_client]);
        Ok(cached)
    }

    fn retire(&self, evicted: &[Arc<Client>]) {
        if evicted.is_empty() {
            return;
        }
        for client in evicted {
            self.closer.schedule(client);
        }
        self.closer.reap();
    }
}

#[path = "llm_client_cache/tests.rs"]
mod tests;
