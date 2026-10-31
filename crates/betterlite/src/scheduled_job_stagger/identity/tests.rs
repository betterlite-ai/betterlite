#![cfg(test)]

use std::collections::BTreeMap;

use super::{IdentityEnvironment, SystemIdentityEnvironment, resolve_stagger_identity};

struct FakeIdentityEnvironment {
    hostname: Option<String>,
    random_identity: String,
    process_id: u32,
}

impl IdentityEnvironment for FakeIdentityEnvironment {
    fn hostname(&self) -> Option<String> {
        self.hostname.clone()
    }

    fn random_identity(&self) -> String {
        self.random_identity.clone()
    }

    fn process_id(&self) -> u32 {
        self.process_id
    }
}

#[test]
fn configured_value_wins_over_the_environment() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), "pod-from-env".to_owned());
    let identity = resolve_stagger_identity(
        Some("configured-identity"),
        &environment,
        &SystemIdentityEnvironment,
    );
    assert!(identity.starts_with("configured-identity:"));
}

#[test]
fn pod_name_wins_over_hostname() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), "pod-7".to_owned());
    environment.insert("HOSTNAME".to_owned(), "host-7".to_owned());
    let identity = resolve_stagger_identity(None, &environment, &SystemIdentityEnvironment);
    assert!(identity.starts_with("pod-7:"));
}

#[test]
fn hostname_is_used_when_pod_name_is_absent() {
    let mut environment = BTreeMap::new();
    environment.insert("HOSTNAME".to_owned(), "host-7".to_owned());
    let identity = resolve_stagger_identity(None, &environment, &SystemIdentityEnvironment);
    assert!(identity.starts_with("host-7:"));
}

#[test]
fn identity_always_ends_with_the_injected_process_id() {
    let dependencies = FakeIdentityEnvironment {
        hostname: Some("fake-host".to_owned()),
        random_identity: "fake-uuid".to_owned(),
        process_id: 4242,
    };
    let identity = resolve_stagger_identity(Some("configured"), &BTreeMap::new(), &dependencies);
    assert_eq!(identity, "configured:4242");
}

#[test]
fn empty_configured_value_falls_back_to_the_environment() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), "pod-7".to_owned());
    let identity = resolve_stagger_identity(Some(""), &environment, &SystemIdentityEnvironment);
    assert!(identity.starts_with("pod-7:"));
}

#[test]
fn empty_environment_values_are_skipped() {
    let mut environment = BTreeMap::new();
    environment.insert("POD_NAME".to_owned(), String::new());
    environment.insert("HOSTNAME".to_owned(), String::new());
    let dependencies = FakeIdentityEnvironment {
        hostname: Some("fake-host".to_owned()),
        random_identity: "fake-uuid".to_owned(),
        process_id: 1,
    };
    let identity = resolve_stagger_identity(None, &environment, &dependencies);
    assert_eq!(identity, "fake-host:1");
}

#[test]
fn falls_back_to_a_random_identity_when_the_hostname_lookup_fails() {
    let dependencies = FakeIdentityEnvironment {
        hostname: None,
        random_identity: "fake-uuid".to_owned(),
        process_id: 1,
    };
    let identity = resolve_stagger_identity(None, &BTreeMap::new(), &dependencies);
    assert_eq!(identity, "fake-uuid:1");
}

#[test]
fn falls_back_to_a_resolvable_host_when_nothing_else_is_set() {
    let identity = resolve_stagger_identity(None, &BTreeMap::new(), &SystemIdentityEnvironment);
    assert!(identity.contains(':'));
    assert!(!identity.starts_with(':'));
}
