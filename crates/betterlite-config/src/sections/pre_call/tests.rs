#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::pre_call::Settings;

fn settings(source: &str) -> Result<Settings, ConfigError> {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    loader.load(source).map(|config| config.sections.pre_call)
}

#[rstest]
#[case::absent("general_settings: {}", None)]
#[case::explicit_null("general_settings:\n  always_include_stream_usage: null", None)]
#[case::on("general_settings:\n  always_include_stream_usage: true", Some(true))]
#[case::off("general_settings:\n  always_include_stream_usage: false", Some(false))]
fn loads_always_include_stream_usage(#[case] source: &str, #[case] expected: Option<bool>) {
    assert_eq!(
        settings(source).unwrap().always_include_stream_usage,
        expected
    );
}

#[test]
fn rejects_a_text_value_for_the_boolean_setting() {
    let source = "general_settings:\n  always_include_stream_usage: maybe";
    let error = settings(source).unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn defaults_to_unset_and_an_empty_alias_map() {
    let loaded = settings("model_list: []").unwrap();
    assert_eq!(loaded, Settings::default());
    assert_eq!(loaded.always_include_stream_usage, None);
    assert!(loaded.model_alias_map.is_empty());
}

#[test]
fn loads_the_model_alias_map() {
    let source = "litellm_settings:\n  model_alias_map:\n    gpt-4: gpt-4o\n    fast: gpt-4o-mini";
    let expected: BTreeMap<String, String> = [("gpt-4", "gpt-4o"), ("fast", "gpt-4o-mini")]
        .into_iter()
        .map(|(alias, target)| (alias.to_owned(), target.to_owned()))
        .collect();
    assert_eq!(settings(source).unwrap().model_alias_map, expected);
}

#[test]
fn an_explicit_null_alias_map_is_empty() {
    let source = "litellm_settings:\n  model_alias_map: null";
    assert!(settings(source).unwrap().model_alias_map.is_empty());
}
