use std::collections::BTreeSet;

use crate::reservation_settlement::handle::get_reserved_counter_keys;

use super::support::{entry, handle};

#[test]
fn every_distinct_counter_key_is_collected() {
    let reservation = handle(
        "10",
        vec![
            entry("spend:key:k1", "5"),
            entry("spend:key:k2", "5"),
            entry("spend:key:k1", "1"),
        ],
        "0",
    );
    let keys = get_reserved_counter_keys(&reservation);
    let expected = BTreeSet::from(["spend:key:k1".to_owned(), "spend:key:k2".to_owned()]);
    assert_eq!(keys, expected);
}

#[test]
fn an_entry_without_a_counter_key_contributes_none() {
    let mut keyless = entry("spend:key:k1", "5");
    keyless.counter_key = None;
    let reservation = handle("5", vec![keyless, entry("spend:key:k2", "5")], "0");
    let expected = BTreeSet::from(["spend:key:k2".to_owned()]);
    assert_eq!(get_reserved_counter_keys(&reservation), expected);
}

#[test]
fn an_empty_reservation_has_no_keys() {
    let reservation = handle("0", Vec::new(), "0");
    assert!(get_reserved_counter_keys(&reservation).is_empty());
}
