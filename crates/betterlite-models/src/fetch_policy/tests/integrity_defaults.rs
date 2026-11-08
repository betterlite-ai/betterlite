use super::support;
use crate::fetch_policy::integrity::{IntegrityPolicy, MAX_SHRINK_RATIO, MIN_MODEL_COUNT};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let policy = IntegrityPolicy::default();
    assert_eq!(policy.min_model_count, MIN_MODEL_COUNT);
    assert_eq!(policy.min_model_count, 50);
    assert_eq!(policy.max_shrink_ratio, MAX_SHRINK_RATIO);
    assert_eq!(policy.max_shrink_ratio, support::decimal("0.5"));
}
