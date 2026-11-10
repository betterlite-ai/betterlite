#![cfg(test)]

use rstest::rstest;

use super::{DEFAULT_TIMEOUT_SECONDS, resolve_passthrough_timeout};

#[rstest]
#[case::endpoint_beats_setting(Some(12.5_f64), 30.0_f64, 12.5_f64)]
#[case::endpoint_zero_is_used(Some(0.0_f64), 30.0_f64, 0.0_f64)]
#[case::setting_without_endpoint(None, 30.0_f64, 30.0_f64)]
#[case::setting_zero_is_used(None, 0.0_f64, 0.0_f64)]
fn the_endpoint_value_wins_over_the_setting(
    #[case] endpoint: Option<f64>,
    #[case] setting: f64,
    #[case] expected: f64,
) {
    assert_eq!(
        resolve_passthrough_timeout(endpoint, setting).to_bits(),
        expected.to_bits()
    );
}

#[test]
fn the_default_is_six_hundred_seconds() {
    assert_eq!(DEFAULT_TIMEOUT_SECONDS.to_bits(), 600.0_f64.to_bits());
}
