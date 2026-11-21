use std::sync::{Mutex, PoisonError};

use indexmap::IndexMap;

// _MAX_STALE_ALIAS_WARNING_KEYS: a client sending a distinct stale alias per request would otherwise grow the set
pub(crate) const MAX_WARNING_KEYS: usize = 10_000;

// _warn_stale_team_alias_once keeps the keys it has warned about in one process-wide OrderedDict
#[derive(Debug, Default)]
pub(crate) struct WarnOnce {
    seen: Mutex<IndexMap<String, ()>>,
}

impl WarnOnce {
    // true the first time a key is seen, false afterwards; the oldest key goes once the cap is passed
    pub(crate) fn first_seen(&self, key: String) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.contains_key(&key) {
            return false;
        }
        seen.insert(key, ());
        while seen.len() > MAX_WARNING_KEYS {
            seen.shift_remove_index(0);
        }
        drop(seen);
        true
    }
}
