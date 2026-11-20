use rstest::rstest;

use crate::health_cycle::rss::{format_rss_mb, rss_mb_for_log};

#[rstest]
#[case::zero(Some(0.0_f64), "0.00")]
#[case::one_and_a_half(Some(1.5_f64), "1.50")]
#[case::large(Some(1234.5678_f64), "1234.57")]
#[case::small(Some(0.004_f64), "0.00")]
fn a_measurement_renders_with_two_decimals(#[case] rss: Option<f64>, #[case] expected: &str) {
    assert_eq!(format_rss_mb(rss), expected);
}

#[test]
fn a_missing_measurement_renders_as_unknown() {
    assert_eq!(format_rss_mb(None), "unknown");
}

#[cfg(target_os = "linux")]
#[test]
fn the_live_measurement_renders_as_a_number() {
    let rendered = rss_mb_for_log();
    assert_ne!(rendered, "unknown");
    let parsed: f64 = rendered.parse().expect("the log holds a decimal number");
    assert!(parsed >= 0.0_f64);
    assert_eq!(rendered.split('.').nth(1).map(str::len), Some(2));
}
