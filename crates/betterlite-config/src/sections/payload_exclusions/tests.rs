#![cfg(test)]

use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use std::collections::BTreeMap;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .payload_exclusions)
}

#[rstest]
#[case::defaults("", Vec::new())]
#[case::empty_list(
    "litellm_settings:\n  standard_logging_payload_excluded_fields: []\n",
    Vec::new()
)]
#[case::null_value(
    "litellm_settings:\n  standard_logging_payload_excluded_fields: null\n",
    Vec::new()
)]
#[case::one_field(
    "litellm_settings:\n  standard_logging_payload_excluded_fields: [messages]\n",
    vec!["messages"]
)]
#[case::several_fields(
    "litellm_settings:\n  standard_logging_payload_excluded_fields: [messages, response, error_str]\n",
    vec!["messages", "response", "error_str"]
)]
fn payload_exclusions_reads_excluded_fields(#[case] source: &str, #[case] expected: Vec<&str>) {
    let settings = load(source, &[]).expect("the excluded fields load");
    assert_eq!(settings.excluded_fields, expected);
    assert_eq!(settings.max_redaction_depth, 10);
}

#[rstest]
#[case::text("litellm_settings:\n  standard_logging_payload_excluded_fields: messages\n")]
#[case::object("litellm_settings:\n  standard_logging_payload_excluded_fields: {messages: true}\n")]
#[case::number_member("litellm_settings:\n  standard_logging_payload_excluded_fields: [1]\n")]
fn payload_exclusions_rejects_a_value_that_is_not_a_list_of_strings(#[case] source: &str) {
    let error = load(source, &[]).expect_err("a non-list value is a config error");
    assert!(matches!(error, ConfigError::InvalidScalar { .. }));
    assert!(
        error
            .to_string()
            .contains("litellm_settings.standard_logging_payload_excluded_fields")
    );
}

#[rstest]
#[case::default(&[], 10)]
#[case::configured(&[(MAX_DEPTH_VARIABLE, "3")], 3)]
#[case::negative(&[(MAX_DEPTH_VARIABLE, "-2")], -2)]
#[case::empty_is_unset(&[(MAX_DEPTH_VARIABLE, "")], 10)]
#[case::surrounding_spaces_trimmed(&[(MAX_DEPTH_VARIABLE, " 4 ")], 4)]
fn payload_exclusions_reads_the_max_redaction_depth(
    #[case] environment: &[(&str, &str)],
    #[case] expected: i64,
) {
    let settings = load("", environment).expect("the depth loads");
    assert_eq!(settings.max_redaction_depth, expected);
}

#[rstest]
#[case::text("abc")]
#[case::float("1.5")]
#[case::signs("--1")]
fn payload_exclusions_rejects_a_depth_that_is_not_an_integer(#[case] value: &str) {
    let error = load("", &[(MAX_DEPTH_VARIABLE, value)]).expect_err("a non-integer depth fails");
    assert!(matches!(error, ConfigError::Environment { .. }));
    assert!(error.to_string().contains(MAX_DEPTH_VARIABLE));
}
