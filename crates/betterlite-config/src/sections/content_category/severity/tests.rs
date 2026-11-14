#![cfg(test)]

use rstest::rstest;

#[test]
fn severity_order_is_low_medium_high() {
    super::tests::severity_thresholds_follow_low_medium_high_order();
}

#[rstest]
#[case::low_low("low", "low", true)]
#[case::low_medium("low", "medium", false)]
#[case::low_high("low", "high", false)]
#[case::medium_low("medium", "low", true)]
#[case::medium_medium("medium", "medium", true)]
#[case::medium_high("medium", "high", false)]
#[case::high_low("high", "low", true)]
#[case::high_medium("high", "medium", true)]
#[case::high_high("high", "high", true)]
#[case::unknown_severity("unknown", "low", true)]
#[case::unknown_threshold("low", "unknown", false)]
fn severity_threshold_combinations(
    #[case] severity: &str,
    #[case] threshold: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        super::loading::should_apply_severity(severity, threshold),
        expected
    );
}
