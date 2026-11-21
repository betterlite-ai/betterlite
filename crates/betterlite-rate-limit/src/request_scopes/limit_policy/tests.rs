#![cfg(test)]

use rstest::rstest;

use super::{get_enforced_limit, should_enforce_rate_limit};

#[rstest]
#[case::dynamic_failing(Some("dynamic"), true, true)]
#[case::dynamic_healthy(Some("dynamic"), false, false)]
#[case::absent_healthy(None, false, true)]
#[case::guaranteed_healthy(Some("guaranteed_throughput"), false, true)]
#[case::best_effort_healthy(Some("best_effort_throughput"), false, true)]
#[case::guaranteed_failing(Some("guaranteed_throughput"), true, true)]
fn only_a_dynamic_type_can_relax(
    #[case] limit_type: Option<&str>,
    #[case] model_has_failures: bool,
    #[case] expected: bool,
) {
    assert_eq!(
        should_enforce_rate_limit(limit_type, model_has_failures),
        expected
    );
}

#[rstest]
#[case::dynamic_failing(Some(10), Some("dynamic"), true, Some(10))]
#[case::dynamic_healthy(Some(10), Some("dynamic"), false, None)]
#[case::absent_type_healthy(Some(10), None, false, Some(10))]
#[case::absent_type_failing(Some(10), None, true, Some(10))]
#[case::zero_failing(Some(0), Some("dynamic"), true, Some(0))]
#[case::zero_healthy(Some(0), Some("dynamic"), false, None)]
fn an_enforced_limit_keeps_its_value(
    #[case] limit_value: Option<i64>,
    #[case] limit_type: Option<&str>,
    #[case] model_has_failures: bool,
    #[case] expected: Option<i64>,
) {
    assert_eq!(
        get_enforced_limit(limit_value, limit_type, model_has_failures),
        expected
    );
}

#[rstest]
#[case::dynamic_failing(Some("dynamic"), true)]
#[case::dynamic_healthy(Some("dynamic"), false)]
#[case::absent(Some("guaranteed_throughput"), true)]
fn an_absent_value_stays_absent(
    #[case] limit_type: Option<&str>,
    #[case] model_has_failures: bool,
) {
    assert_eq!(
        get_enforced_limit(None, limit_type, model_has_failures),
        None
    );
}
