#![cfg(test)]

use super::manager::pod_id;
use super::support::{SHARED_AT, World, instant};

#[test]
fn the_id_is_the_epoch_millisecond_it_started_at() {
    assert_eq!(pod_id(instant("2024-01-01T00:00:00Z")), "pod_1704067200000");
    assert_eq!(
        pod_id(instant("2020-02-29T12:34:56.789Z")),
        "pod_1582979696789"
    );
}

#[test]
fn two_startups_in_one_second_differ_by_their_millisecond() {
    let early = pod_id(instant("2024-01-01T00:00:00.001Z"));
    let late = pod_id(instant("2024-01-01T00:00:00.999Z"));
    assert_eq!(early, "pod_1704067200001");
    assert_eq!(late, "pod_1704067200999");
    assert_ne!(early, late);
}

#[test]
fn a_pre_epoch_reading_stays_negative() {
    assert_eq!(pod_id(instant("1969-12-31T23:59:59.999Z")), "pod_-1");
}

#[test]
fn the_manager_keeps_the_id_it_was_built_with() {
    let world = World::new(instant(SHARED_AT));
    let later = World::new(instant("2024-01-01T00:00:01Z"));
    assert_eq!(world.manager().pod_id, "pod_1704067200000");
    assert_eq!(later.manager().pod_id, "pod_1704067201000");
}
