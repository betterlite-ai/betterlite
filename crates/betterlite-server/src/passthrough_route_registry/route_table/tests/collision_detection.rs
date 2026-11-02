use std::collections::BTreeSet;

use super::super::RouteTable;

fn methods(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn exact_path_and_method_match_is_a_collision() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    assert_eq!(table.colliding_routes("/foo", &methods(&["GET"])).len(), 1);
}

#[test]
fn different_path_is_not_a_collision() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    assert!(
        table
            .colliding_routes("/bar", &methods(&["GET"]))
            .is_empty()
    );
}

#[test]
fn disjoint_methods_are_not_a_collision() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    assert!(
        table
            .colliding_routes("/foo", &methods(&["POST"]))
            .is_empty()
    );
}

#[test]
fn overlapping_method_sets_are_a_collision() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET", "POST"]), false);
    assert_eq!(
        table
            .colliding_routes("/foo", &methods(&["POST", "PUT"]))
            .len(),
        1
    );
}

#[test]
fn collision_with_only_lazy_owned_routes_does_not_block() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), true);
    assert!(!table.is_path_registered("/foo", &methods(&["GET"])));
}

#[test]
fn collision_with_a_non_lazy_owned_route_blocks() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    assert!(table.is_path_registered("/foo", &methods(&["GET"])));
}

#[test]
fn two_routes_at_the_same_path_are_distinct_by_identity() {
    let mut table = RouteTable::default();
    let first = table.install("/foo".to_owned(), methods(&["GET"]), false);
    let second = table.install("/foo".to_owned(), methods(&["GET"]), true);
    assert_ne!(first, second);
    assert_eq!(table.colliding_routes("/foo", &methods(&["GET"])).len(), 2);
}
