use std::time::Duration;

pub const DEBOUNCE: Duration = Duration::from_secs(1);
pub const JITTER_MAX_NANOS: u64 = 5_000_000_000;
pub const MIN_RESYNC_INTERVAL: Duration = Duration::from_secs(10);
pub const POLL_TIMEOUT: Duration = Duration::from_secs(1);
pub const BACKOFF_INITIAL: Duration = Duration::from_secs(5);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

pub struct ConfigSyncSettings {
    pub debounce: Duration,
    pub jitter_max_nanos: u64,
    pub min_resync_interval: Duration,
    pub poll_timeout: Duration,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
}

impl Default for ConfigSyncSettings {
    fn default() -> Self {
        Self {
            debounce: DEBOUNCE,
            jitter_max_nanos: JITTER_MAX_NANOS,
            min_resync_interval: MIN_RESYNC_INTERVAL,
            poll_timeout: POLL_TIMEOUT,
            backoff_initial: BACKOFF_INITIAL,
            backoff_max: BACKOFF_MAX,
        }
    }
}
