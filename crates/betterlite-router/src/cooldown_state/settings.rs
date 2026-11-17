// the settings the cooldown and health services read, keyed by the configuration keys that carry them
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub default_cooldown_time: f64,
    pub redis_read_interval_seconds: f64,
    pub enable_health_check_routing: bool,
    pub health_check_staleness_threshold: f64,
    pub health_check_ignore_transient_errors: bool,
}

// LiteLLM's DEFAULT_COOLDOWN_TIME_SECONDS, DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS and the interval times its staleness multiplier
impl Default for Settings {
    fn default() -> Self {
        Self {
            default_cooldown_time: 5.0,
            redis_read_interval_seconds: 1.0,
            enable_health_check_routing: false,
            health_check_staleness_threshold: 600.0,
            health_check_ignore_transient_errors: false,
        }
    }
}
