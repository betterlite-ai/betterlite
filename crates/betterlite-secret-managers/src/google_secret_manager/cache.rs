use std::{collections::BTreeMap, time::Duration};

use secrecy::SecretString;

const CAPACITY: usize = 200;

struct Entry {
    value: Option<SecretString>,
    expires_at: Duration,
}

pub enum CacheLookup {
    Miss,
    Hit(Option<SecretString>),
}

pub struct SecretCache {
    entries: BTreeMap<String, Entry>,
    time_to_live: Duration,
}

impl SecretCache {
    #[must_use]
    pub const fn new(time_to_live: Duration) -> Self {
        Self {
            entries: BTreeMap::new(),
            time_to_live,
        }
    }

    pub fn lookup(&mut self, secret_name: &str, now: Duration) -> CacheLookup {
        let expired = self
            .entries
            .get(secret_name)
            .is_some_and(|entry| now > entry.expires_at);
        if expired {
            self.entries.remove(secret_name);
        }
        self.entries
            .get(secret_name)
            .map_or(CacheLookup::Miss, |entry| {
                CacheLookup::Hit(entry.value.clone())
            })
    }

    pub fn store(&mut self, secret_name: &str, value: Option<SecretString>, now: Duration) {
        let live_expiry = self
            .entries
            .get(secret_name)
            .map(|entry| entry.expires_at)
            .filter(|expires_at| *expires_at >= now);
        if live_expiry.is_none() && self.entries.len() >= CAPACITY {
            self.entries.retain(|_, entry| entry.expires_at >= now);
        }
        while live_expiry.is_none() && self.entries.len() >= CAPACITY {
            let Some(earliest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(name, _)| name.clone())
            else {
                break;
            };
            self.entries.remove(&earliest);
        }
        let expires_at = live_expiry.unwrap_or_else(|| now.saturating_add(self.time_to_live));
        self.entries
            .insert(secret_name.to_owned(), Entry { value, expires_at });
    }
}

mod tests;
