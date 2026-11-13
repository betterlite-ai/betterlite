use rstest::rstest;

use super::support;
use crate::fetch_policy::integrity::count_model_entries;

#[rstest]
#[case::empty(&[], 0)]
#[case::reserved_only(&["sample_spec", "fallback_generalizations"], 0)]
#[case::mixed(&["sample_spec", "a", "fallback_generalizations", "b"], 2)]
#[case::lookalike(
    &["sample_specification", "fallback_generalizations_extended"],
    2
)]
#[case::ordinary(&["only"], 1)]
fn catalog_fetch_policy_behavior_tests_behavior(#[case] names: &[&str], #[case] expected: usize) {
    assert_eq!(count_model_entries(&support::models(names)), expected);
}
