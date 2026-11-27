use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::login_throttle::TRUSTED_PROXY_RANGES_KEY;
use tracing::warn;

// LiteLLM memoizes both startup warnings with functools.cache, per num_workers for the first and once for the second; the server owns one cache, so a restart is what lets them through again.
#[derive(Debug, Default)]
pub struct WarningCache {
    emitted: Mutex<BTreeSet<String>>,
}

impl WarningCache {
    pub fn warn_counters_per_worker(&self, num_workers: &str) {
        let key = format!("counters:{num_workers}");
        let first = self
            .emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
        if first {
            warn!(
                "Running {num_workers} workers but Redis is not configured. Failed Admin UI sign-in attempts are counted per worker, so the effective limits are {num_workers} times the configured values. Configure Redis to share one count across workers."
            );
        }
    }

    pub fn warn_source_login_limit_is_off(&self) {
        let first = self
            .emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert("source".to_owned());
        if first {
            warn!(
                "{TRUSTED_PROXY_RANGES_KEY} is not set or not a valid list of ranges, so failed Admin UI sign-in attempts are limited per source address and username only. Set it to the address ranges of the proxies in front of LiteLLM, or to an empty list when clients connect directly, to also limit each source address across usernames."
            );
        }
    }
}
