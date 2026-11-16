#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::straiker::delivery::fixtures::{raw, settings};
use crate::straiker::delivery::settings::{SettingsError, StraikerSettings};

#[rstest]
#[case::negative(-5_i64, 0_i64)]
#[case::zero(0_i64, 0_i64)]
#[case::positive(7_i64, 7_i64)]
fn retries_are_clamped_to_zero_or_more(#[case] configured: i64, #[case] expected: i64) {
    let mut args = raw();
    args.max_retries = Some(configured);
    assert_eq!(settings(&args).max_retries, expected);
}

#[rstest]
#[case::negative(-1.5_f64, 0.0_f64)]
#[case::zero(0.0_f64, 0.0_f64)]
#[case::positive(0.25_f64, 0.25_f64)]
#[case::nan(f64::NAN, 0.0_f64)]
fn the_initial_backoff_is_clamped_to_zero_or_more(#[case] configured: f64, #[case] expected: f64) {
    let mut args = raw();
    args.initial_backoff = Some(configured);
    assert_eq!(
        settings(&args).initial_backoff.to_bits(),
        expected.to_bits()
    );
}

#[test]
fn a_max_backoff_below_the_initial_is_raised_to_it() {
    let mut args = raw();
    args.initial_backoff = Some(0.5_f64);
    args.max_backoff = Some(0.25_f64);
    assert_eq!(settings(&args).max_backoff.to_bits(), 0.5_f64.to_bits());
}

#[test]
fn a_max_backoff_without_the_initial_is_raised_to_it() {
    let mut args = raw();
    args.initial_backoff = Some(5.0_f64);
    assert_eq!(settings(&args).max_backoff.to_bits(), 5.0_f64.to_bits());
}

#[test]
fn a_max_backoff_above_the_initial_is_kept() {
    let mut args = raw();
    args.initial_backoff = Some(0.5_f64);
    args.max_backoff = Some(3.0_f64);
    assert_eq!(settings(&args).max_backoff.to_bits(), 3.0_f64.to_bits());
}

#[test]
fn an_infinite_initial_backoff_is_rejected() {
    let mut args = raw();
    args.initial_backoff = Some(f64::INFINITY);
    let error = StraikerSettings::resolve(&args).expect_err("the backoff is infinite");
    assert_eq!(
        error.to_string(),
        "initial_backoff must be a finite number of seconds; got inf"
    );
    assert!(matches!(error, SettingsError::InitialBackoff { .. }));
}

#[test]
fn an_infinite_max_backoff_is_rejected() {
    let mut args = raw();
    args.max_backoff = Some(f64::INFINITY);
    let error = StraikerSettings::resolve(&args).expect_err("the backoff is infinite");
    assert_eq!(
        error.to_string(),
        "max_backoff must be a finite number of seconds; got inf"
    );
    assert!(matches!(error, SettingsError::MaxBackoff { .. }));
}

#[rstest]
#[case::unset(None, Duration::from_secs(5))]
#[case::zero(Some(0.0_f64), Duration::ZERO)]
#[case::fractional(Some(2.5_f64), Duration::from_millis(2500))]
fn the_timeout_is_read_as_seconds(#[case] configured: Option<f64>, #[case] expected: Duration) {
    let mut args = raw();
    args.timeout = configured;
    assert_eq!(settings(&args).timeout, expected);
}

#[rstest]
#[case::negative(-1.0_f64, "-1")]
#[case::infinite(f64::INFINITY, "inf")]
fn an_unusable_timeout_is_rejected(#[case] configured: f64, #[case] shown: &str) {
    let mut args = raw();
    args.timeout = Some(configured);
    let error = StraikerSettings::resolve(&args).expect_err("the timeout is unusable");
    assert_eq!(
        error.to_string(),
        format!("timeout must be a finite number of seconds; got {shown}")
    );
    assert!(matches!(error, SettingsError::Timeout { .. }));
}

#[rstest]
#[case::unset(None, 512_usize * 1024)]
#[case::zero(Some(0_i64), 0_usize)]
#[case::exact(Some(1024_i64), 1024_usize)]
fn the_payload_limit_is_read_as_bytes(#[case] configured: Option<i64>, #[case] expected: usize) {
    let mut args = raw();
    args.max_payload_bytes = configured;
    assert_eq!(settings(&args).max_payload_bytes, expected);
}

#[test]
fn a_negative_payload_limit_is_rejected() {
    let mut args = raw();
    args.max_payload_bytes = Some(-1);
    let error = StraikerSettings::resolve(&args).expect_err("the limit is negative");
    assert_eq!(
        error.to_string(),
        "max_payload_bytes must be a non-negative integer; got -1"
    );
    assert!(matches!(
        error,
        SettingsError::MaxPayloadBytes { value: -1 }
    ));
}
