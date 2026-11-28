#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_router::router_runtime_settings::UPDATABLE_ROUTER_SETTINGS;

use super::super::ConfigUpdateError;
use super::super::SettingsMap;
use super::super::router_settings::{self, MANAGED_OUTSIDE_CONFIG, ROUTER_INIT_ARGUMENTS};
use super::support::{object, parsed};

#[test]
fn accepts_every_router_init_argument_and_runtime_setting() {
    let raw: SettingsMap = ROUTER_INIT_ARGUMENTS
        .iter()
        .chain(UPDATABLE_ROUTER_SETTINGS.iter())
        .map(|name| ((*name).to_owned(), JsonValue::Null))
        .filter(|(name, _)| !MANAGED_OUTSIDE_CONFIG.contains(&name.as_str()))
        .collect();
    assert!(router_settings::unsupported(&raw).is_empty());
    router_settings::check(&raw).unwrap();
}

#[test]
fn refuses_the_settings_managed_outside_the_config() {
    let raw: SettingsMap = MANAGED_OUTSIDE_CONFIG
        .iter()
        .map(|name| ((*name).to_owned(), JsonValue::Null))
        .collect();
    let mut expected: Vec<String> = MANAGED_OUTSIDE_CONFIG
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    expected.sort_unstable();
    assert_eq!(router_settings::unsupported(&raw), expected);
}

#[test]
fn refuses_unknown_and_typed_but_unsupported_settings_in_sorted_order() {
    let raw = object(r#"{"weights": {}, "nope": 1, "model_list": []}"#);
    assert_eq!(
        router_settings::unsupported(&raw),
        vec![
            "model_list".to_owned(),
            "nope".to_owned(),
            "weights".to_owned()
        ]
    );
    let error = router_settings::check(&raw).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unsupported router settings: model_list, nope, weights are not valid router settings"
    );
}

#[test]
fn accepts_valid_routing_groups() {
    let raw = object(
        r#"{"routing_groups": [{"group_name": "g", "models": ["m"], "routing_strategy": "simple-shuffle"}]}"#,
    );
    router_settings::check(&raw).unwrap();
}

#[test]
fn refuses_duplicate_group_names() {
    let raw = object(
        r#"{"routing_groups": [
            {"group_name": "g", "models": ["m1"], "routing_strategy": "simple-shuffle"},
            {"group_name": "g", "models": ["m2"], "routing_strategy": "simple-shuffle"}
        ]}"#,
    );
    let error = router_settings::check(&raw).unwrap_err();
    assert!(matches!(&error, ConfigUpdateError::RoutingGroups(_)));
    assert!(error.to_string().contains("group names must be unique"));
}

#[test]
fn refuses_a_routing_groups_that_is_not_a_list() {
    let raw = object(r#"{"routing_groups": "g"}"#);
    let error = router_settings::check(&raw).unwrap_err();
    assert_eq!(error.to_string(), "routing_groups must be a list");
}

#[test]
fn refuses_a_group_that_is_not_an_object() {
    let raw = object(r#"{"routing_groups": [1]}"#);
    let error = router_settings::check(&raw).unwrap_err();
    assert_eq!(error.to_string(), "routing_groups entries must be objects");
}

#[test]
fn updates_put_the_typed_fields_first_and_skip_null_raw_keys() {
    let typed = object(r#"{"timeout": 1.0}"#);
    let raw = object(r#"{"timeout": 9.0, "enable_weighted_failover": true, "num_retries": null}"#);
    let updates = router_settings::updates(typed, Some(&raw));
    assert_eq!(updates.get("timeout"), Some(&parsed("1.0")));
    assert_eq!(
        updates.get("enable_weighted_failover"),
        Some(&JsonValue::Bool(true))
    );
    assert!(!updates.contains_key("num_retries"));
}

#[test]
fn updates_without_a_raw_object_are_the_typed_fields() {
    let typed = object(r#"{"num_retries": 2}"#);
    assert_eq!(router_settings::updates(typed.clone(), None), typed);
}
