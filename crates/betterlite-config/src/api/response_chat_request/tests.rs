#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn source(yaml: Option<&str>) -> String {
    yaml.map_or_else(
        || "model_list: []\n".to_owned(),
        |value| format!("model_list: []\nlitellm_settings:\n  reasoning_auto_summary: {value}\n"),
    )
}

fn settings(source: &str, environment: &BTreeMap<String, String>) -> Settings {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .expect("the config loads")
    .sections
    .response_chat_request
}

#[rstest]
#[case::unset(&[], None, false)]
#[case::flag_false(&[], Some("false"), false)]
#[case::flag_true(&[], Some("true"), true)]
#[case::env_upper_true(&[("LITELLM_REASONING_AUTO_SUMMARY", "TRUE")], None, true)]
#[case::env_lower_true(&[("LITELLM_REASONING_AUTO_SUMMARY", "true")], None, true)]
#[case::env_padded(&[("LITELLM_REASONING_AUTO_SUMMARY", " true ")], None, false)]
#[case::env_one(&[("LITELLM_REASONING_AUTO_SUMMARY", "1")], None, false)]
#[case::env_garbage(&[("LITELLM_REASONING_AUTO_SUMMARY", "garbage")], None, false)]
#[case::flag_false_env_true(
    &[("LITELLM_REASONING_AUTO_SUMMARY", "true")],
    Some("false"),
    true
)]
#[case::flag_true_env_false(
    &[("LITELLM_REASONING_AUTO_SUMMARY", "false")],
    Some("true"),
    true
)]
fn loads_reasoning_summary_settings(
    #[case] environment: &[(&str, &str)],
    #[case] yaml: Option<&str>,
    #[case] expected: bool,
) {
    let loaded = settings(&source(yaml), &variables(environment));
    assert_eq!(loaded.reasoning_auto_summary, expected);
}

#[rstest]
#[case::word("maybe")]
#[case::quoted("'true'")]
fn rejects_an_invalid_yaml_boolean(#[case] yaml: &str) {
    let error = ConfigLoader {
        lookup: variables(&[]),
    }
    .load(&source(Some(yaml)))
    .expect_err("the loader rejects the value");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}

#[test]
fn recognizes_the_reasoning_auto_summary_key() {
    let config = ConfigLoader {
        lookup: variables(&[]),
    }
    .load(&source(Some("true")))
    .expect("the recognized key loads");
    assert!(config.sections.response_chat_request.reasoning_auto_summary);
    let error = ConfigLoader {
        lookup: variables(&[]),
    }
    .load(
        "model_list: []\nlitellm_settings:\n  reasoning_auto_summary: true\n  vendor_flag: true\n",
    )
    .expect_err("an unclaimed sibling key stays unsupported");
    assert!(matches!(error, ConfigError::Unsupported { .. }), "{error}");
}
