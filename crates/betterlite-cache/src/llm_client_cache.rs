use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::evicted_client_closer::{
    Clock, EvictableClient, EvictedClientCloser, current_event_loop_id,
};

pub const DEFAULT_MAX_SIZE_IN_MEMORY: usize = 200;
pub const DEFAULT_TTL_SECONDS: f64 = 600.0;

struct Entry<Client: EvictableClient> {
    client: Arc<Client>,
    expiry: f64,
}

pub struct LlmClientCache<Client: EvictableClient> {
    capacity: usize,
    default_ttl: f64,
    clock: Clock,
    closer: Arc<EvictedClientCloser<Client>>,
    entries: Mutex<BTreeMap<String, Entry<Client>>>,
}

fn event_loop_key(key: &str) -> String {
    let Some(loop_id) = current_event_loop_id() else {
        return key.to_owned();
    };
    format!("{key}-{loop_id}")
}

// InMemoryCache.evict_cache: expired entries first, then the earliest expiry while the cache is full
fn eviction_keys<Client>(
    entries: &BTreeMap<String, Entry<Client>>,
    capacity: usize,
    now: f64,
) -> Vec<String>
where
    Client: EvictableClient,
{
    let mut keys = Vec::new();
    let mut live: Vec<(&String, f64)> = Vec::new();
    for (key, entry) in entries {
        if entry.expiry <= now {
            keys.push(key.clone());
        } else {
            live.push((key, entry.expiry));
        }
    }
    live.sort_by(|(left_key, left_expiry), (right_key, right_expiry)| {
        left_expiry
            .total_cmp(right_expiry)
            .then_with(|| left_key.cmp(right_key))
    });
    if live.len() >= capacity {
        let excess = live.len().saturating_sub(capacity).saturating_add(1);
        keys.extend(
            live.iter()
                .take(excess)
                .map(|(key, _expiry)| (*key).clone()),
        );
    }
    keys
}

fn evict<Client>(
    entries: &mut BTreeMap<String, Entry<Client>>,
    capacity: usize,
    now: f64,
    evicted: &mut Vec<Arc<Client>>,
) where
    Client: EvictableClient,
{
    for key in eviction_keys(entries, capacity, now) {
        if let Some(entry) = entries.remove(&key) {
            evicted.push(entry.client);
        }
    }
}

impl<Client: EvictableClient> LlmClientCache<Client> {
    #[must_use]
    pub fn new(
        capacity: Option<usize>,
        default_ttl: Option<f64>,
        clock: Clock,
        closer: Arc<EvictedClientCloser<Client>>,
    ) -> Self {
        Self {
            capacity: capacity.unwrap_or(DEFAULT_MAX_SIZE_IN_MEMORY),
            default_ttl: default_ttl.unwrap_or(DEFAULT_TTL_SECONDS),
            clock,
            closer,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn set_cache(
        &self,
        key: &str,
        client: Arc<Client>,
        ttl: Option<f64>,
        litellm_owned_client: bool,
    ) {
        if litellm_owned_client {
            self.closer.mark_owned(&client);
        }
        if self.capacity == 0 {
            return;
        }
        let decorated = event_loop_key(key);
        let now = (self.clock)();
        let evicted = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            let mut evicted = Vec::new();
            evict(&mut entries, self.capacity, now, &mut evicted);
            // whatever survived the expiry sweep is live, and a live entry keeps its own deadline
            let expiry = entries.get(&decorated).map_or_else(
                || now.mul_add(1.0_f64, ttl.unwrap_or(self.default_ttl)),
                |entry| entry.expiry,
            );
            entries.insert(decorated, Entry { client, expiry });
            evicted
        };
        self.retire(&evicted);
    }

    pub async fn async_set_cache(
        &self,
        key: &str,
        client: Arc<Client>,
        ttl: Option<f64>,
        litellm_owned_client: bool,
    ) {
        self.set_cache(key, client, ttl, litellm_owned_client);
        self.closer.reap_async().await;
    }

    #[must_use]
    pub fn get_cache(&self, key: &str) -> Option<Arc<Client>> {
        let decorated = event_loop_key(key);
        self.closer.reap();
        self.lookup(&decorated)
    }

    #[must_use]
    pub async fn async_get_cache(&self, key: &str) -> Option<Arc<Client>> {
        let decorated = event_loop_key(key);
        self.closer.reap_async().await;
        self.lookup(&decorated)
    }

    fn lookup(&self, decorated: &str) -> Option<Arc<Client>> {
        let now = (self.clock)();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let expired = entries
            .get(decorated)
            .is_some_and(|entry| now > entry.expiry);
        if expired {
            let evicted = entries.remove(decorated).map(|entry| entry.client);
            drop(entries);
            if let Some(client) = evicted {
                self.retire(&[client]);
            }
            return None;
        }
        entries
            .get(decorated)
            .map(|entry| Arc::clone(&entry.client))
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
