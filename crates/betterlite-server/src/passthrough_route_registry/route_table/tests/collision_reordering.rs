use std::collections::BTreeSet;

use super::super::RouteTable;

fn methods(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn lazy_collision_does_not_block_addition() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), true);
    assert!(table.add_route_if_not_exists("/foo".to_owned(), methods(&["GET"])));
    assert_eq!(table.order().len(), 2);
}

#[test]
fn added_route_is_reordered_ahead_of_the_colliding_lazy_route() {
    let mut table = RouteTable::default();
    let lazy = table.install("/foo".to_owned(), methods(&["GET"]), true);
    table.add_route_if_not_exists("/foo".to_owned(), methods(&["GET"]));
    let order = table.order();
    let lazy_index = order.iter().position(|id| *id == lazy);
    assert_eq!(lazy_index, Some(1));
}

#[test]
fn reorder_preserves_relative_order_of_other_routes() {
    let mut table = RouteTable::default();
    let first = table.install("/x".to_owned(), methods(&["GET"]), false);
    let lazy = table.install("/foo".to_owned(), methods(&["GET"]), true);
    let last = table.install("/z".to_owned(), methods(&["GET"]), false);
    table.add_route_if_not_exists("/foo".to_owned(), methods(&["GET"]));
    let order = table.order();
    let positions: Vec<usize> = [first, lazy, last]
        .iter()
        .filter_map(|id| order.iter().position(|candidate| candidate == id))
        .collect();
    assert_eq!(positions, vec![0, 2, 3]);
}

#[test]
fn non_colliding_route_is_appended_without_reorder() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    table.add_route_if_not_exists("/bar".to_owned(), methods(&["GET"]));
    assert_eq!(table.order().len(), 2);
}

#[test]
fn collision_with_a_non_lazy_route_blocks_addition_without_reorder() {
    let mut table = RouteTable::default();
    table.install("/foo".to_owned(), methods(&["GET"]), false);
    assert!(!table.add_route_if_not_exists("/foo".to_owned(), methods(&["GET"])));
    assert_eq!(table.order().len(), 1);
}
