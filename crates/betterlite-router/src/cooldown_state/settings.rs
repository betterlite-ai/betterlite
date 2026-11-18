#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub default_cooldown_time: f64,
    pub redis_read_interval_seconds: f64,
    pub enable_health_check_routing: bool,
    pub staleness_threshold: f64,
    pub ignore_transient_errors: bool,
}

// LiteLLM's DEFAULT_COOLDOWN_TIME_SECONDS, DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS and the interval times its staleness multiplier
impl Default for Settings {
    fn default() -> Self {
        Self {
            default_cooldown_time: 5.0,
            redis_read_interval_seconds: 1.0,
            enable_health_check_routing: false,
            staleness_threshold: 600.0,
            ignore_transient_errors: false,
        }
    }
}
