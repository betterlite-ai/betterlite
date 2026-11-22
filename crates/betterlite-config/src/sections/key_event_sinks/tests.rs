#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_secret_managers::secret_contracts::options::SecretTags;
use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::from([("SINK_PREFIX".to_owned(), "env/keys".to_owned())]),
    }
    .load(source)
    .map(|config| config.sections.key_event_sinks)
}

#[test]
fn key_management_defaults() {
    let settings = load("general_settings: {}\n").unwrap();
    assert!(!settings.store_virtual_keys);
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
    assert_eq!(settings.description, None);
    assert_eq!(settings.tags, None);
    assert!(settings.alerting.is_empty());
}

#[rstest]
#[case::literal("vault/keys", "vault/keys")]
#[case::empty("''", "")]
#[case::environment("os.environ/SINK_PREFIX", "env/keys")]
fn configured_prefix_tags_description(#[case] prefix: &str, #[case] expected: &str) {
    let source = format!(
        "general_settings:\n  key_management_settings:\n    prefix_for_stored_virtual_keys: {prefix}\n    description: managed keys\n    tags: {{Team: core}}\n"
    );
    let settings = load(&source).unwrap();
    assert_eq!(settings.prefix_for_stored_virtual_keys, expected);
    assert_eq!(settings.description.as_deref(), Some("managed keys"));
    assert_eq!(
        settings.tags,
        Some(SecretTags::Mapping(BTreeMap::from([(
            "Team".to_owned(),
            "core".to_owned()
        )])))
    );
}

#[rstest]
#[case::explicit_false("false", false)]
#[case::null("null", false)]
#[case::explicit_true("true", true)]
fn store_virtual_keys_false_null_true(#[case] value: &str, #[case] expected: bool) {
    let source =
        format!("general_settings:\n  key_management_settings:\n    store_virtual_keys: {value}\n");
    assert_eq!(load(&source).unwrap().store_virtual_keys, expected);
}

#[rstest]
#[case::email("general_settings:\n  alerting: [slack, email]\n", &["slack", "email"])]
#[case::null("general_settings:\n  alerting: null\n", &[])]
#[case::empty("general_settings:\n  alerting: []\n", &[])]
fn legacy_alerting_nested_binding(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(load(source).unwrap().alerting, expected);
}

#[test]
fn legacy_alerting_top_level_key_is_not_read() {
    let error = load("alerting: [email]\n").unwrap_err();
    assert!(matches!(&error, ConfigError::Unsupported { paths } if paths == &["alerting"]));
}

#[rstest]
#[case::text_switch(
    "key_management_settings: {store_virtual_keys: maybe}",
    "invalid_scalar"
)]
#[case::quoted_switch(
    "key_management_settings: {store_virtual_keys: 'true'}",
    "invalid_scalar"
)]
#[case::access_mode("key_management_settings: {access_mode: bogus}", "invalid_scalar")]
#[case::string_alerting("alerting: email", "yaml")]
#[case::number_alerting("alerting: 5", "yaml")]
fn invalid_settings_rejected(#[case] setting: &str, #[case] category: &str) {
    let source = format!("general_settings:\n  {setting}\n");
    assert_eq!(load(&source).unwrap_err().category(), category);
}
