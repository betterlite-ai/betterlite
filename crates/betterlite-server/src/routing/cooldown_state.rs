use betterlite_config::sections::cooldown_state as config;
use betterlite_router::cooldown_state::settings::Settings;

mod tests;

// Router.__init__: the proxy hands the four general_settings keys and the configured cooldown time to its caches
#[must_use]
pub const fn cooldown_settings(
    section: &config::CooldownStateSettings,
    default_cooldown_time: f64,
) -> Settings {
    Settings {
        default_cooldown_time,
        redis_read_interval_seconds: section.read_interval_seconds,
        enable_health_check_routing: section.enable_health_check_routing,
        staleness_threshold: section.staleness_threshold,
        ignore_transient_errors: section.ignore_transient_errors,
    }
}
