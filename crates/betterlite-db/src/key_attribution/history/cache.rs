use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use crate::key_attribution::metadata::KeyMetadata;

use super::window::{HistoryWindow, spend_log_cache_key};

pub const MAX_ITEMS: usize = 10_000;
pub const TTL: Duration = Duration::from_secs(600);
pub const MISS_TTL: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Entry {
    metadata: KeyMetadata,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct MetadataCache {
    pub capacity: usize,
    pub entries: Mutex<BTreeMap<String, Entry>>,
}

impl Default for MetadataCache {
    fn default() -> Self {
        Self {
            capacity: MAX_ITEMS,
            entries: Mutex::new(BTreeMap::new()),
        }
    }
}

impl MetadataCache {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<KeyMetadata> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let stored = entries
            .get(key)
            .map(|entry| (entry.expires_at >= now, entry.metadata.clone()));
        match stored {
            Some((true, metadata)) => Some(metadata),
            Some((false, _)) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn set(&self, key: &str, metadata: KeyMetadata, ttl: Duration) {
        if self.capacity == 0 {
            return;
        }
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        entries.retain(|_, entry| entry.expires_at > now);
        while entries.len() >= self.capacity {
            let oldest = entries
                .iter()
                .min_by(|left, right| {
                    (left.1.expires_at, left.0).cmp(&(right.1.expires_at, right.0))
                })
                .map(|(name, _)| name.clone());
            let Some(name) = oldest else { break };
            entries.remove(&name);
        }
        // A live entry keeps its deadline, as LiteLLM's in-memory cache does
        let expires_at = entries.get(key).map_or_else(
            || now.checked_add(ttl).unwrap_or(now),
            |existing| existing.expires_at,
        );
        entries.insert(
            key.to_owned(),
            Entry {
                metadata,
                expires_at,
            },
        );
    }

    #[must_use]
    pub fn cached(
        &self,
        digests: &BTreeSet<String>,
        window: &HistoryWindow,
    ) -> BTreeMap<String, KeyMetadata> {
        digests
            .iter()
            .filter_map(|digest| {
                self.get(&spend_log_cache_key(digest, window))
                    .map(|metadata| (digest.clone(), metadata))
            })
            .collect()
    }

    pub fn remember(&self, digest: &str, window: &HistoryWindow, found: Option<&KeyMetadata>) {
        let key = spend_log_cache_key(digest, window);
        if let Some(metadata) = found {
            self.set(&key, metadata.clone(), TTL);
            return;
        }
        let missed_before = format!("{key}:missed-before");
        if self.get(&missed_before).is_some() {
            self.set(&key, KeyMetadata::default(), TTL);
            return;
        }
        self.set(&key, KeyMetadata::default(), MISS_TTL);
        self.set(&missed_before, KeyMetadata::default(), TTL);
    }
}
