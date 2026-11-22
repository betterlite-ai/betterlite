#![cfg(test)]

use super::agent_ids;
use crate::agent_permission_handler::error::GrantError;
use crate::agent_permission_handler::tests::support::{World, group_row, set, strings, world};

#[test]
fn agents_from_every_group_are_unioned() {
    let world = world(true);
    let mut state = world.lock();
    state
        .groups
        .insert(String::from("u1"), group_row("u1", &["a", "b"]));
    state
        .groups
        .insert(String::from("u2"), group_row("u2", &["b", "c"]));
    drop(state);
    let found = agent_ids(Some(&world), &strings(&["u1", "u2"]), false).unwrap();
    assert_eq!(found, set(&["a", "b", "c"]));
}

#[test]
fn a_missing_lookup_returns_nothing() {
    let found = agent_ids(None::<&World>, &strings(&["u1"]), false).unwrap();
    assert!(found.is_empty());
}

#[test]
fn no_group_ids_returns_nothing() {
    let world = world(true);
    let found = agent_ids(Some(&world), &[], true).unwrap();
    assert!(found.is_empty());
}

#[test]
fn a_strict_failure_is_an_error() {
    let world = world(true);
    let error = agent_ids(Some(&world), &strings(&["u1"]), true).unwrap_err();
    let GrantError::Lookup(_) = error else {
        panic!("lookup error expected, got {error:?}");
    };
}

#[test]
fn a_non_strict_failure_is_skipped() {
    let world = world(true);
    world
        .lock()
        .groups
        .insert(String::from("u1"), group_row("u1", &["a"]));
    let found = agent_ids(Some(&world), &strings(&["u1", "gone"]), false).unwrap();
    assert_eq!(found, set(&["a"]));
}
