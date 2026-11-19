#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_pyhost::json_values::JsonObject;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::PluginRegistry;
use crate::plugin_routes::error::PluginError;
use crate::plugin_routes::test_support::settings;

fn names(registry: &PluginRegistry) -> Vec<String> {
    registry.plugins.read().unwrap().keys().cloned().collect()
}

#[test]
fn replaces_the_registry_whole() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha"},{"name":"beta","display_name":"Beta","url":"http://beta"}]}"#,
        ))
        .unwrap();
    assert_eq!(names(&registry), ["alpha", "beta"]);
    let plugins = registry.plugins.read().unwrap();
    assert_eq!(plugins["alpha"].display_name, None);
    assert!(plugins["alpha"].plugin_key.is_none());
    assert_eq!(plugins["beta"].display_name.as_deref(), Some("Beta"));
}

#[test]
fn replaces_rather_than_merges_on_reload() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha"},{"name":"beta","url":"http://beta"}]}"#,
        ))
        .unwrap();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"beta","url":"http://beta"}]}"#,
        ))
        .unwrap();
    assert_eq!(names(&registry), ["beta"]);
}

#[test]
fn clears_the_registry_when_the_key_is_absent() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha"}]}"#,
        ))
        .unwrap();
    registry.replace_from_settings(&settings("{}")).unwrap();
    assert!(names(&registry).is_empty());
}

#[test]
fn clears_the_registry_when_plugins_is_not_an_array() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha"}]}"#,
        ))
        .unwrap();
    registry
        .replace_from_settings(&settings(r#"{"plugins":{"name":"beta"}}"#))
        .unwrap();
    assert!(names(&registry).is_empty());
}

#[test]
fn keeps_the_first_position_and_the_later_value_for_a_duplicate_name() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://first"},{"name":"beta","url":"http://beta"},{"name":"alpha","url":"http://later"}]}"#,
        ))
        .unwrap();
    assert_eq!(names(&registry), ["alpha", "beta"]);
    assert_eq!(
        registry.plugins.read().unwrap()["alpha"].url,
        "http://later"
    );
}

#[test]
fn keeps_the_old_registry_when_an_entry_is_invalid() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha"}]}"#,
        ))
        .unwrap();
    let result = registry.replace_from_settings(&settings(
        r#"{"plugins":[{"name":"beta","url":"http://beta"},{"name":"broken"}]}"#,
    ));
    assert!(matches!(
        result,
        Err(PluginError::InvalidEntry { index: 1, .. })
    ));
    assert_eq!(names(&registry), ["alpha"]);
}

#[test]
fn names_the_index_and_the_field_in_the_error() {
    let registry = PluginRegistry::default();
    let result = registry.replace_from_settings(&settings(
        r#"{"plugins":[{"name":"alpha","url":"http://alpha"},{"name":"broken"}]}"#,
    ));
    let Err(error) = result else {
        panic!("the second entry has no url");
    };
    let text = error.to_string();
    assert!(text.contains("entry 1"), "{text}");
    assert!(text.contains("url"), "{text}");
}

#[test]
fn rejects_an_entry_with_a_wrong_field_type() {
    let registry = PluginRegistry::default();
    let result =
        registry.replace_from_settings(&settings(r#"{"plugins":[{"name":"alpha","url":5}]}"#));
    assert!(matches!(
        result,
        Err(PluginError::InvalidEntry { index: 0, .. })
    ));
    assert!(names(&registry).is_empty());
}

#[test]
fn parses_the_plugin_key_as_a_secret() {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(
            r#"{"plugins":[{"name":"alpha","url":"http://alpha","plugin_key":"sk-plugin"}]}"#,
        ))
        .unwrap();
    let plugins = registry.plugins.read().unwrap();
    let key = plugins["alpha"].plugin_key.as_ref().unwrap();
    assert_eq!(key.expose_secret(), "sk-plugin");
}

#[rstest]
fn replays_the_plugin_registry_corpus(#[files("fuzz/corpus/plugin_registry/*")] path: PathBuf) {
    let text = fs::read_to_string(path).expect("a corpus seed should be readable");
    let Ok(settings) = serde_json::from_str::<JsonObject>(&text) else {
        return;
    };
    let registry = PluginRegistry::default();
    drop(registry.replace_from_settings(&settings));
}
