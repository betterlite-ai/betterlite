#![cfg(test)]

use betterlite_config::sections::latency_observations::Args;
use betterlite_router::latency_observations::settings::Settings;

use super::latency_settings;

#[test]
fn absent_arguments_keep_the_declared_defaults() {
    assert_eq!(latency_settings(&Args::default()), Settings::default());
}

#[test]
fn configured_arguments_replace_the_defaults() {
    let args = Args {
        ttl_seconds: Some(60.0),
        lowest_latency_buffer: Some(0.1),
        max_latency_list_size: Some(3),
        ttft_percentile: Some(0.5),
    };
    assert_eq!(
        latency_settings(&args),
        Settings {
            ttl_seconds: 60.0,
            lowest_latency_buffer: 0.1,
            max_latency_list_size: 3,
            ttft_percentile: Some(0.5),
        }
    );
}
