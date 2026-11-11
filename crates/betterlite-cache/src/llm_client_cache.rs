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
            clock,
        )?;
        Ok(Self {
            closer,
            inner: Mutex::new(Inner {
                markers,
                values: BTreeMap::new(),
            }),
        })
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
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        // LiteLLM overwrites a live entry without closing the client it replaces
        let (evicted, _replaced) = store_marked(
            &mut inner.markers,
            &mut inner.values,
            decorated,
            client,
            ttl,
        )?;
        drop(guard);
        self.retire(&evicted);
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
