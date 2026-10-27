#![cfg(test)]

use determinator::rules::DeterminatorRules;

use crate::affected::{RULES, arguments};

#[test]
fn no_affected_package_prints_nothing() {
    assert!(arguments(3, Vec::new()).is_empty());
}

#[test]
fn every_member_affected_runs_the_workspace() {
    assert_eq!(
        arguments(2, vec!["betterlite-http", "betterlite"]),
        vec!["--workspace".to_owned()]
    );
}

#[test]
fn some_members_affected_print_sorted_package_flags() {
    assert_eq!(
        arguments(5, vec!["betterlite-oracle", "betterlite-a2a"]),
        vec!["-p betterlite-a2a -p betterlite-oracle".to_owned()]
    );
}

#[test]
fn the_rules_parse() {
    DeterminatorRules::parse(RULES).unwrap();
}
