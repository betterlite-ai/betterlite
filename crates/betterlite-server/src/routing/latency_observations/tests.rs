#![cfg(test)]

use betterlite_config::sections::latency_observations::Args;
use betterlite_router::latency_observations::settings::Settings;

use super::latency_settings;

#[test]
fn absent_arguments_keep_the_declared_defaults() {
    assert_eq!(
        latency_settings(&Args::default()),
        Settings {
            ttl_seconds: 3_600.0_f64,
            lowest_latency_buffer: 0.0_f64,
            max_latency_list_size: 10,
            ttft_percentile: None,
        }
    );
}

#[test]
fn configured_arguments_replace_the_defaults() {
    let args = Args {
        ttl_seconds: Some(60.0_f64),
        lowest_latency_buffer: Some(0.1_f64),
        max_latency_list_size: Some(3),
        ttft_percentile: Some(0.5_f64),
    };
    assert_eq!(
        latency_settings(&args),
        Settings {
            ttl_seconds: 60.0_f64,
            lowest_latency_buffer: 0.1_f64,
            max_latency_list_size: 3,
            ttft_percentile: Some(0.5_f64),
        }
    );
}
