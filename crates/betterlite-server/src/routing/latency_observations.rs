use betterlite_config::sections::latency_observations as config;
use betterlite_router::latency_observations::settings::Settings;

mod tests;

// LowestLatencyLoggingHandler.__init__: RoutingArgs keeps the configured argument or the one it declares
#[must_use]
pub fn latency_settings(section: &config::Args) -> Settings {
    let defaults = Settings::default();
    Settings {
        ttl_seconds: section.ttl_seconds.unwrap_or(defaults.ttl_seconds),
        lowest_latency_buffer: section
            .lowest_latency_buffer
            .unwrap_or(defaults.lowest_latency_buffer),
        max_latency_list_size: section
            .max_latency_list_size
            .unwrap_or(defaults.max_latency_list_size),
        ttft_percentile: section.ttft_percentile,
    }
}
