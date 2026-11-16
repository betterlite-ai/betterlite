#![cfg(test)]

use rstest::rstest;

use super::{UsageStatus, UsageTrend, status_from_fail_rate, trend_from_comparison};

#[rstest]
#[case::zero(0.0_f64, UsageStatus::Healthy)]
#[case::at_warning_threshold(5.0_f64, UsageStatus::Healthy)]
#[case::just_over_warning(5.000_001_f64, UsageStatus::Warning)]
#[case::inside_warning(10.0_f64, UsageStatus::Warning)]
#[case::at_critical_threshold(15.0_f64, UsageStatus::Warning)]
#[case::just_over_critical(15.000_001_f64, UsageStatus::Critical)]
#[case::blocked_everything(100.0_f64, UsageStatus::Critical)]
fn status_thresholds_are_strict(#[case] fail_rate: f64, #[case] expected: UsageStatus) {
    assert_eq!(status_from_fail_rate(fail_rate), expected);
}

#[rstest]
#[case::rise(10.500_1_f64, 10.0_f64, UsageTrend::Up)]
#[case::fall(9.499_9_f64, 10.0_f64, UsageTrend::Down)]
#[case::half_point_rise(10.5_f64, 10.0_f64, UsageTrend::Stable)]
#[case::half_point_fall(9.5_f64, 10.0_f64, UsageTrend::Stable)]
#[case::inside_the_band(10.4_f64, 10.0_f64, UsageTrend::Stable)]
#[case::down_from_high(0.0_f64, 80.0_f64, UsageTrend::Down)]
fn trend_moves_only_past_half_a_point(
    #[case] current: f64,
    #[case] previous: f64,
    #[case] expected: UsageTrend,
) {
    assert_eq!(trend_from_comparison(current, previous), expected);
}

#[rstest]
#[case::no_previous_rate(80.0_f64, 0.0_f64)]
#[case::both_zero(0.0_f64, 0.0_f64)]
#[case::negative_previous(80.0_f64, -1.0_f64)]
fn trend_stays_stable_without_a_previous_rate(#[case] current: f64, #[case] previous: f64) {
    assert_eq!(trend_from_comparison(current, previous), UsageTrend::Stable);
}

#[test]
fn status_and_trend_serialize_as_lowercase_strings() {
    assert_eq!(
        serde_json::to_string(&UsageStatus::Critical).unwrap(),
        "\"critical\""
    );
    assert_eq!(
        serde_json::to_string(&UsageStatus::Warning).unwrap(),
        "\"warning\""
    );
    assert_eq!(
        serde_json::to_string(&UsageStatus::Healthy).unwrap(),
        "\"healthy\""
    );
    assert_eq!(serde_json::to_string(&UsageTrend::Up).unwrap(), "\"up\"");
    assert_eq!(
        serde_json::to_string(&UsageTrend::Down).unwrap(),
        "\"down\""
    );
    assert_eq!(
        serde_json::to_string(&UsageTrend::Stable).unwrap(),
        "\"stable\""
    );
}
