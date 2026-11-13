use crate::newrelic_extraction::ids::{SystemUuids, UuidSource as _};

#[test]
fn newrelic_extraction_tests_behavior_system_ids_are_random_v4_uuids() {
    let first = SystemUuids.next_id();
    assert_eq!(first.get_version_num(), 4);
    assert!(!first.is_nil());
    assert_ne!(first, SystemUuids.next_id());
}
