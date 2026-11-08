use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::time::Instant;

use super::Clock;

const CAPACITY: usize = 200;

pub(crate) struct Entry {
    value: SecretString,
    expiry: Instant,
}

pub(crate) struct TtlCache {
    pub(crate) entries: Mutex<BTreeMap<String, Entry>>,
    pub(crate) ttl: Duration,
    pub(crate) clock: Clock,
}

impl TtlCache {
    pub(crate) fn get(&self, key: &str) -> Option<SecretString> {
        let now = (self.clock)();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.get(key).is_some_and(|entry| now > entry.expiry) {
            entries.remove(key);
        }
        entries.get(key).map(|entry| entry.value.clone())
    }

    // An unexpired entry keeps its first expiry when overwritten, as LiteLLM's InMemoryCache does.
    pub(crate) fn set(&self, key: &str, value: SecretString) {
        let now = (self.clock)();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, entry| now < entry.expiry);
        while entries.len() >= CAPACITY {
            let earliest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.expiry)
                .map(|(name, _)| name.clone());
            let Some(name) = earliest else { break };
            entries.remove(&name);
        }
        let kept_expiry = entries.get(key).map(|entry| entry.expiry);
        let Some(expiry) = kept_expiry.or_else(|| now.checked_add(self.ttl)) else {
            return;
        };
        entries.insert(key.to_owned(), Entry { value, expiry });
    }

    pub(crate) fn delete(&self, key: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}
