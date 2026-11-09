use crate::newrelic_extraction::ids::{IdSource as _, SystemIds};

#[test]
fn newrelic_extraction_tests_behavior_system_ids_are_random_v4_uuids() {
    let first = SystemIds.next_id();
    assert_eq!(first.get_version_num(), 4);
    assert!(!first.is_nil());
    assert_ne!(first, SystemIds.next_id());
}
