#![cfg(test)]

use std::collections::BTreeMap;

use super::EnvironmentLookup as _;

#[test]
fn lookup_returns_the_matching_value() {
    let mut env = BTreeMap::new();
    env.insert("REDIS_HOST".to_owned(), "cache.internal".to_owned());
    assert_eq!(env.lookup("REDIS_HOST"), Some("cache.internal".to_owned()));
}

#[test]
fn lookup_returns_none_for_an_unset_name() {
    let env: BTreeMap<String, String> = BTreeMap::new();
    assert_eq!(env.lookup("REDIS_HOST"), None);
}
