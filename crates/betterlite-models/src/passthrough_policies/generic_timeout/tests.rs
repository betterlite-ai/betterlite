#![cfg(test)]

use rstest::rstest;

use super::{DEFAULT_TIMEOUT_SECONDS, resolve_passthrough_timeout};

#[rstest]
#[case::endpoint_beats_setting(Some(12.5_f64), Some(30.0_f64), 12.5_f64)]
#[case::endpoint_zero_is_used(Some(0.0_f64), Some(30.0_f64), 0.0_f64)]
#[case::endpoint_beats_the_default(Some(90.0_f64), None, 90.0_f64)]
#[case::setting_beats_the_default(None, Some(30.0_f64), 30.0_f64)]
#[case::setting_zero_is_used(None, Some(0.0_f64), 0.0_f64)]
#[case::setting_negative_is_used(None, Some(-5.0_f64), -5.0_f64)]
#[case::nothing_configured(None, None, DEFAULT_TIMEOUT_SECONDS)]
fn precedence_uses_the_first_configured_value(
    #[case] endpoint: Option<f64>,
    #[case] setting: Option<f64>,
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
    assert_eq!(
        resolve_passthrough_timeout(None, None).to_bits(),
        600.0_f64.to_bits()
    );
}
