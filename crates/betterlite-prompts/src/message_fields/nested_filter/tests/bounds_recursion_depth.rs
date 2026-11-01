use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let at_bound_levels = MAX_RECURSE_DEPTH;
    let over_bound_levels = MAX_RECURSE_DEPTH + 1;
    let at_bound = nest(at_bound_levels, "secret");
    let over_bound = nest(over_bound_levels, "secret");

    let filtered_at_bound = filter_value_from_dict(&at_bound, "secret", 0);
    let filtered_over_bound = filter_value_from_dict(&over_bound, "secret", 0);

    assert!(!leaf_present_at(
        &filtered_at_bound,
        at_bound_levels,
        "secret"
    ));
    assert!(leaf_present_at(
        &filtered_over_bound,
        over_bound_levels,
        "secret"
    ));
}
