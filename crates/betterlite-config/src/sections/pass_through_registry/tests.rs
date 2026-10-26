#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

fn root(source: &str) -> String {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("config should load")
    .sections
    .pass_through_registry
    .server_root_path
}

#[test]
fn defaults_to_empty_root() {
    assert_eq!(root("{}"), "");
}

#[test]
fn loads_explicit_root_values() {
    assert_eq!(root("SERVER_ROOT_PATH: /proxy"), "/proxy");
    assert_eq!(root("SERVER_ROOT_PATH: /proxy/"), "/proxy/");
}
