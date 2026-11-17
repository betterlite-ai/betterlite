#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(yaml: &str, variables: &[(&str, &str)]) -> bool {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(yaml)
        .expect("the config loads")
        .sections
        .streaming_logging
        .disable
}

#[test]
fn loads_disable_flag() {
    assert!(load(
        "litellm_settings: {disable_streaming_logging: true}",
        &[]
    ));
    assert!(!load(
        "litellm_settings: {disable_streaming_logging: false}",
        &[]
    ));
    assert!(!load("litellm_settings: {}", &[]));
}

#[rstest]
#[case::enabled("true", true)]
#[case::disabled("false", false)]
#[case::empty_takes_the_default("", false)]
#[case::padded(" yes ", true)]
fn the_disable_flag_reads_an_environment_reference(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(
        load(
            "litellm_settings: {disable_streaming_logging: os.environ/DISABLE_STREAMING_LOGGING}",
            &[("DISABLE_STREAMING_LOGGING", value)],
        ),
        expected
    );
}

#[test]
fn an_unset_environment_reference_is_a_load_error() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {disable_streaming_logging: os.environ/DISABLE_STREAMING_LOGGING}")
    .unwrap_err();
    assert!(error.to_string().contains("DISABLE_STREAMING_LOGGING"));
}

#[rstest]
#[case::number("litellm_settings: {disable_streaming_logging: 3}")]
#[case::literal_string("litellm_settings: {disable_streaming_logging: \"true\"}")]
fn a_wrong_type_is_a_load_error(#[case] yaml: &str) {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Yaml { .. } | ConfigError::Environment { .. }
    ));
}

#[test]
fn null_takes_the_default() {
    assert!(!load(
        "litellm_settings: {disable_streaming_logging: null}",
        &[]
    ));
}
