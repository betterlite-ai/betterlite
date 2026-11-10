#![cfg(test)]

use std::collections::BTreeMap;

use crate::provider_defaults_a::environment::first_set;

#[test]
fn first_set_skips_empty_values_and_keeps_declaration_order() {
    let env = BTreeMap::from([
        ("ALPHA".to_owned(), String::new()),
        ("BETA".to_owned(), "beta-value".to_owned()),
        ("GAMMA".to_owned(), "gamma-value".to_owned()),
    ]);
    assert_eq!(
        first_set(&env, &["ALPHA", "BETA", "GAMMA"]),
        Some("beta-value".to_owned())
    );
}

#[test]
fn first_set_returns_none_when_every_variable_is_absent_or_empty() {
    let env = BTreeMap::from([("ALPHA".to_owned(), String::new())]);
    assert_eq!(first_set(&env, &["ALPHA", "MISSING"]), None);
}
