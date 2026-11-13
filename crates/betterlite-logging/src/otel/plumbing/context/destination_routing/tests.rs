#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::EnvironmentLookup;

use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::plumbing::context::destination_routing::{
    TENANT_DESTINATION_MODE_ENV, destination_backends, destinations_are_additive,
    suppressed_backends,
};

fn destination(callback_name: Option<&str>) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: callback_name.map(str::to_owned),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}

fn environment(entry: Option<&str>) -> BTreeMap<String, String> {
    entry
        .map(|value| BTreeMap::from([(TENANT_DESTINATION_MODE_ENV.to_owned(), value.to_owned())]))
        .unwrap_or_default()
}

fn names(backends: BTreeSet<String>) -> Vec<String> {
    backends.into_iter().collect()
}

#[test]
fn override_mode_suppresses_every_named_destination_backend() {
    let destinations = [
        destination(Some("arize")),
        destination(Some("langfuse_otel")),
    ];
    assert_eq!(
        names(suppressed_backends(&destinations, false)),
        vec!["arize".to_owned(), "langfuse_otel".to_owned()]
    );
}

#[test]
fn additive_mode_suppresses_nothing() {
    let destinations = [destination(Some("arize"))];
    assert!(suppressed_backends(&destinations, true).is_empty());
}

#[test]
fn destinations_without_a_callback_name_are_not_backends() {
    let destinations = [destination(None), destination(Some("arize"))];
    assert_eq!(
        names(destination_backends(&destinations)),
        vec!["arize".to_owned()]
    );
}

#[test]
fn the_configured_mode_wins_over_the_environment() {
    let environment = environment(Some("additive"));
    assert!(!destinations_are_additive(Some("override"), &environment));
}

#[test]
fn the_environment_supplies_the_mode_when_the_config_has_none() {
    let environment = environment(Some("additive"));
    assert!(destinations_are_additive(None, &environment));
    assert!(!destinations_are_additive(
        None,
        &BTreeMap::<String, String>::new()
    ));
}

#[test]
fn an_empty_configured_mode_is_used_as_given() {
    let environment = environment(Some("additive"));
    assert!(!destinations_are_additive(Some(""), &environment));
}

#[test]
fn mode_matching_trims_spaces_and_ignores_case() {
    let empty = BTreeMap::<String, String>::new();
    assert!(destinations_are_additive(Some("  ADDITIVE "), &empty));
    assert!(destinations_are_additive(
        None,
        &environment(Some(" Additive "))
    ));
}

#[test]
fn an_unset_mode_is_override() {
    assert!(!destinations_are_additive(
        None,
        &BTreeMap::<String, String>::new()
    ));
    assert!(!destinations_are_additive(
        None,
        &environment(Some("something-else"))
    ));
}

#[test]
fn the_environment_lookup_is_the_one_that_is_passed_in() {
    struct EmptyEnvironment;
    impl EnvironmentLookup for EmptyEnvironment {
        fn lookup(&self, _name: &str) -> Option<String> {
            None
        }
    }
    assert!(!destinations_are_additive(None, &EmptyEnvironment));
}
