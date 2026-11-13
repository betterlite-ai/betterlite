use super::support;
use crate::fetch_policy::integrity::{IntegrityError, check_model_count_not_reduced};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let with_markers = support::models(&["sample_spec", "a", "fallback_generalizations", "b"]);
    check_model_count_not_reduced(&with_markers, 0, support::policy(2, "0.5")).unwrap();
    assert!(matches!(
        check_model_count_not_reduced(&with_markers, 0, support::policy(3, "0.5")),
        Err(IntegrityError::TooFewModels {
            count: 2,
            minimum: 3
        })
    ));
}
