#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::json;

use super::JsonValue;

use crate::loader::{ConfigError, ConfigLoader};

#[rstest]
#[case::loads_set_verbose("litellm_settings:\n  set_verbose: true\n", BTreeMap::new(), json!(true))]
#[case::loads_truthy_non_boolean("litellm_settings:\n  set_verbose: 1\n", BTreeMap::new(), json!(1_i64))]
#[case::default("{}\n", BTreeMap::new(), json!(false))]
#[case::explicit_false("litellm_settings:\n  set_verbose: false\n", BTreeMap::new(), json!(false))]
#[case::environment("litellm_settings:\n  set_verbose: os.environ/VERBOSE\n", BTreeMap::from([("VERBOSE".to_owned(), "yes".to_owned())]), json!("yes"))]
#[case::literal_string("litellm_settings:\n  set_verbose: yes\n", BTreeMap::new(), json!("yes"))]
fn verbose_hooks_config_cases(
    #[case] source: &str,
    #[case] lookup: BTreeMap<String, String>,
    #[case] expected: JsonValue,
) {
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("config should load");
    assert_eq!(config.sections.verbose_hooks.set_verbose, expected);
}

#[test]
fn default_settings_disable_verbose() {
    assert_eq!(super::Settings::default().set_verbose, json!(false));
}

#[test]
fn missing_verbose_environment_is_named() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  set_verbose: os.environ/VERBOSE\n")
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "VERBOSE" && path == "litellm_settings.set_verbose")
    );
}

#[test]
fn unrelated_verbose_key_is_rejected() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  set_verbose: true\n  verbose_hook_unknown: false\n")
    .unwrap_err();
    assert!(error.to_string().contains("verbose_hook_unknown"));
}
