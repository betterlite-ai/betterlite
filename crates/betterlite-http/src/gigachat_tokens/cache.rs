use std::collections::HashMap;

use secrecy::SecretString;
use tokio::sync::Mutex;

pub(crate) const MAX_ENTRIES: usize = 200;
pub(crate) const EXPIRY_BUFFER_MILLIS: i64 = 60_000;

struct Entry {
    token: SecretString,
    expires_at: i64,
}

#[derive(Default)]
pub(crate) struct TokenCache {
    entries: Mutex<HashMap<String, Entry>>,
}

impl TokenCache {
    pub(crate) async fn lookup(&self, key: &str, now_millis: i64) -> Option<SecretString> {
        let entries = self.entries.lock().await;
        let live = entries
            .get(key)
            .filter(|entry| now_millis < entry.expires_at.saturating_sub(EXPIRY_BUFFER_MILLIS))
            .map(|entry| entry.token.clone());
        drop(entries);
        live
    }

    pub(crate) async fn insert(
        &self,
        key: String,
        token: SecretString,
        expires_at: i64,
        now_millis: i64,
    ) {
        let mut entries = self.entries.lock().await;
        entries
            .retain(|_, entry| now_millis < entry.expires_at.saturating_sub(EXPIRY_BUFFER_MILLIS));
        while entries.len() >= MAX_ENTRIES {
            let Some(earliest) = entries
                .iter()
                .min_by(|left, right| {
                    left.1
                        .expires_at
                        .cmp(&right.1.expires_at)
                        .then_with(|| left.0.cmp(right.0))
                })
                .map(|(stored, _)| stored.clone())
            else {
                break;
            };
            entries.remove(&earliest);
        }
        match entries.get_mut(&key) {
            Some(entry) => entry.token = token,
            None => {
                entries.insert(key, Entry { token, expires_at });
            }
        }
    }
}
