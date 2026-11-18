use std::time::Duration;

use betterlite_config::sections::routing_state as state_config;
use betterlite_config::sections::usage_v2_accounting as config;
use betterlite_router::routing_state::Settings as CounterSettings;
use betterlite_router::usage_v2_accounting::{SHOULD_BATCH_WRITES, SYNC_INTERVAL};

mod tests;

// Router.__init__ builds the v2 strategy with its routing args and the counter loop it batches its writes through
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub ttl: Duration,
    pub counters: CounterSettings,
}

#[must_use]
pub const fn resolve(section: &config::Settings, state: &state_config::Settings) -> Resolved {
    Resolved {
        ttl: section.ttl,
        counters: CounterSettings {
            sync_interval: Some(SYNC_INTERVAL),
            default_sync_interval: state.default_sync_interval,
            should_batch_redis_writes: SHOULD_BATCH_WRITES,
        },
    }
}
