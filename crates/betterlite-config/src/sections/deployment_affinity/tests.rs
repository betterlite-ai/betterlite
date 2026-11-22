#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::deployment_affinity::Settings;

fn settings(source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .deployment_affinity
}

fn settings_with(variables: &[(&str, &str)], source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .deployment_affinity
}

// every key absent takes the declaration defaults
#[test]
fn the_keys_absent_take_the_defaults() {
    let settings = settings("");
    assert_eq!(settings.optional_pre_call_checks, None);
    assert_eq!(settings.model_group_affinity_config, None);
    assert_eq!(settings.ttl_seconds, 3600);
}

// the three keys the deployment affinity check reads load as given
#[test]
fn the_three_keys_load_as_given() {
    let settings = settings(
        "router_settings:\n  optional_pre_call_checks:\n    - deployment_affinity\n  model_group_affinity_config:\n    gpt-4o:\n      - session_affinity\n  deployment_affinity_ttl_seconds: 120\n",
    );
    assert_eq!(
        settings.optional_pre_call_checks,
        Some(vec!["deployment_affinity".to_owned()])
    );
    let config = settings
        .model_group_affinity_config
        .expect("the config is set");
    assert_eq!(
        config.get("gpt-4o"),
        Some(&vec!["session_affinity".to_owned()])
    );
    assert_eq!(settings.ttl_seconds, 120);
}

// an explicit empty list stays an empty list rather than absence
#[test]
fn an_empty_list_stays_present() {
    let settings = settings("router_settings:\n  optional_pre_call_checks: []\n");
    assert_eq!(settings.optional_pre_call_checks, Some(Vec::new()));
}

// the ttl resolves through the injected environment
#[test]
fn the_ttl_resolves_from_the_environment() {
    let settings = settings_with(
        &[("B324_TTL", "42")],
        "router_settings:\n  deployment_affinity_ttl_seconds: os.environ/B324_TTL\n",
    );
    assert_eq!(settings.ttl_seconds, 42);
}

// a ttl that is not a number stops the load at its path
#[test]
fn a_ttl_that_is_not_a_number_stops_the_load() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load("router_settings:\n  deployment_affinity_ttl_seconds: soon\n")
        .err()
        .expect("the load fails");
    assert!(
        error
            .to_string()
            .contains("deployment_affinity_ttl_seconds")
    );
}

// a negative ttl stops the load too
#[test]
fn a_negative_ttl_stops_the_load() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load("router_settings:\n  deployment_affinity_ttl_seconds: -1\n")
        .err()
        .expect("the load fails");
    assert!(
        error
            .to_string()
            .contains("deployment_affinity_ttl_seconds")
    );
}
