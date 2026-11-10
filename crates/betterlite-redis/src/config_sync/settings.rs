use std::time::Duration;

pub const DEBOUNCE_SECONDS: f64 = 1.0;
pub const JITTER_MAX_SECONDS: f64 = 5.0;
pub const MIN_RESYNC_INTERVAL_SECONDS: f64 = 10.0;
pub const POLL_TIMEOUT_SECONDS: f64 = 1.0;
pub const BACKOFF_INITIAL_SECONDS: f64 = 5.0;
pub const BACKOFF_MAX_SECONDS: f64 = 60.0;

pub struct ConfigSyncSettings {
    pub debounce: Duration,
    pub jitter_max: Duration,
    pub min_resync_interval: Duration,
    pub poll_timeout: Duration,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
}

// LiteLLM's constants are literal whole seconds, so the fallible conversion cannot fail
impl Default for ConfigSyncSettings {
    fn default() -> Self {
        Self {
            debounce: Duration::try_from_secs_f64(DEBOUNCE_SECONDS).unwrap_or_default(),
            jitter_max: Duration::try_from_secs_f64(JITTER_MAX_SECONDS).unwrap_or_default(),
            min_resync_interval: Duration::try_from_secs_f64(MIN_RESYNC_INTERVAL_SECONDS)
                .unwrap_or_default(),
            poll_timeout: Duration::try_from_secs_f64(POLL_TIMEOUT_SECONDS).unwrap_or_default(),
            backoff_initial: Duration::try_from_secs_f64(BACKOFF_INITIAL_SECONDS)
                .unwrap_or_default(),
            backoff_max: Duration::try_from_secs_f64(BACKOFF_MAX_SECONDS).unwrap_or_default(),
        }
    }
}
