pub mod counter;
pub mod delta;
pub mod error;
mod lifecycle;
pub mod metadata;
mod sink;
mod sync;
mod tests;

use std::time::Duration;

// one owner's batch and its join set: LiteLLM drained the whole queue, and the bound keeps a stuck store's backlog finite
pub const MAX_PENDING_DELTAS: usize = 1024;
// the keys a sync reads back from the store; LiteLLM's set was unbounded and grew one key per window
pub const MAX_TRACKED_KEYS: usize = 1024;
// how long a flush, a sync or a full queue's caller waits for the store before the observation is dropped
pub const FLUSH_DEADLINE: Duration = Duration::from_secs(1);
// BaseRoutingStrategy.get_key_pattern_to_sync: no strategy narrows the keys a sync reads back
pub const KEY_PATTERN_TO_SYNC: Option<&str> = None;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub sync_interval: Option<Duration>,
    pub default_sync_interval: Duration,
    pub should_batch_redis_writes: bool,
}

impl Settings {
    // LiteLLM's "default_sync_interval or DEFAULT_REDIS_SYNC_INTERVAL": zero or absent takes the default
    #[must_use]
    pub const fn resolved_sync_interval(&self) -> Duration {
        match self.sync_interval {
            Some(interval) if !interval.is_zero() => interval,
            _ => self.default_sync_interval,
        }
    }
}
