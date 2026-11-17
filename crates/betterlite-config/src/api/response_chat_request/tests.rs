#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Raw, Settings};
use crate::loader::{ConfigError, parse_view};
use crate::sections::Section as _;

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

// the section parses and converts through the same two calls RawSections makes while the loader runs
fn settings(source: &str, environment: &BTreeMap<String, String>) -> Settings {
    let (raw, _ignored) = parse_view::<Raw>(source).expect("the section parses");
    raw.convert(environment).expect("the section converts")
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
    let error = parse_view::<Raw>(&source(Some(yaml))).expect_err("the view rejects the value");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}

#[test]
fn recognizes_the_reasoning_auto_summary_key() {
    let (_raw, ignored) = parse_view::<Raw>(&source(Some("true"))).expect("the section parses");
    assert!(
        !ignored
            .iter()
            .any(|path| path.contains("reasoning_auto_summary")),
        "{ignored:?}"
    );
}
