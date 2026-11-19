#![cfg(test)]

use betterlite_config::sections::latency_routing as config;
use betterlite_router::strategy_dispatch::args::LatencyArgs;

use super::resolve;

#[test]
fn latency_routing_resolve_keeps_the_declared_defaults() {
    assert_eq!(
        resolve(&config::Settings::default()),
        LatencyArgs::default()
    );
}

#[test]
fn latency_routing_resolve_uses_the_configured_selection_arguments() {
    let section = Box::new(config::LatencyRoutingSettings {
        lowest_latency_buffer: Some(0.25_f64),
        ttft_percentile: Some(0.9_f64),
    });
    assert_eq!(
        resolve(&section),
        LatencyArgs {
            lowest_latency_buffer: 0.25_f64,
            ttft_percentile: Some(0.9_f64),
            ..LatencyArgs::default()
        }
    );
}

// an unset percentile stays unset while the configured buffer is read
#[test]
fn latency_routing_resolve_keeps_an_unset_percentile() {
    let section = Box::new(config::LatencyRoutingSettings {
        lowest_latency_buffer: Some(0.5_f64),
        ttft_percentile: None,
    });
    assert_eq!(
        resolve(&section),
        LatencyArgs {
            lowest_latency_buffer: 0.5_f64,
            ..LatencyArgs::default()
        }
    );
}
