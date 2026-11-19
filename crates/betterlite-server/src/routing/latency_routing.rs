use betterlite_config::sections::latency_routing as config;
use betterlite_router::strategy_dispatch::args::LatencyArgs;

mod tests;

// Router.__init__ hands routing_strategy_args to RoutingArgs; a selection reads the buffer and the percentile, the rest keeps the declared defaults
#[must_use]
pub fn resolve(section: &config::Settings) -> LatencyArgs {
    let declared = LatencyArgs::default();
    LatencyArgs {
        lowest_latency_buffer: section
            .lowest_latency_buffer
            .unwrap_or(declared.lowest_latency_buffer),
        ttft_percentile: section.ttft_percentile.or(declared.ttft_percentile),
        ..declared
    }
}
