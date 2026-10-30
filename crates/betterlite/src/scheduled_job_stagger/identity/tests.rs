#![cfg(test)]

use std::collections::BTreeMap;
use std::process;

use super::resolve_stagger_identity;

#[test]
fn configured_value_wins_over_the_environment() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), "pod-from-env".to_owned());
    let identity = resolve_stagger_identity(Some("configured-identity"), &environment);
    assert!(identity.starts_with("configured-identity:"));
}

#[test]
fn pod_name_wins_over_hostname() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), "pod-7".to_owned());
    environment.insert("HOSTNAME".to_owned(), "host-7".to_owned());
    let identity = resolve_stagger_identity(None, &environment);
    assert!(identity.starts_with("pod-7:"));
}

#[test]
fn hostname_is_used_when_pod_name_is_absent() {
    let mut environment = BTreeMap::new();
    environment.insert("HOSTNAME".to_owned(), "host-7".to_owned());
    let identity = resolve_stagger_identity(None, &environment);
    assert!(identity.starts_with("host-7:"));
}

#[test]
fn identity_always_ends_with_this_process_pid() {
    let identity = resolve_stagger_identity(Some("configured"), &BTreeMap::new());
    assert_eq!(identity, format!("configured:{}", process::id()));
}

#[test]
fn falls_back_to_a_resolvable_host_when_nothing_else_is_set() {
    let identity = resolve_stagger_identity(None, &BTreeMap::new());
    assert!(identity.contains(':'));
    assert!(!identity.starts_with(':'));
}
