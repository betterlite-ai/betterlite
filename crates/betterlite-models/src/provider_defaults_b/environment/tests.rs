#![cfg(test)]

use std::collections::BTreeMap;

use super::EnvironmentLookup as _;

#[test]
fn lookup_returns_the_matching_value() {
    let mut env = BTreeMap::new();
    env.insert("MISTRAL_API_KEY".to_owned(), "configured".to_owned());
    assert_eq!(env.lookup("MISTRAL_API_KEY"), Some("configured".to_owned()));
}

#[test]
fn lookup_returns_none_for_an_unset_name() {
    let env: BTreeMap<String, String> = BTreeMap::new();
    assert_eq!(env.lookup("MISTRAL_API_KEY"), None);
}
