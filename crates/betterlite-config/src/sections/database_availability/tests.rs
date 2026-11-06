#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_availability::policy::{
    AllowRequestsOnOutage, PolicyError, allows_requests_during_outage,
};
use rstest::rstest;

use super::Settings;
use crate::loader::ConfigLoader;

fn load(source: &str, variables: &[(&str, &str)]) -> Settings {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .expect("config should load")
    .sections
    .database_availability
}

#[test]
fn default_false() {
    let settings = load("general_settings: {}\n", &[]);
    assert_eq!(
        settings.allow_requests,
        AllowRequestsOnOutage::Boolean(false)
    );
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Ok(false)
    );
}

#[rstest]
#[case::enabled(true)]
#[case::disabled(false)]
fn boolean_values(#[case] expected: bool) {
    let source = format!("general_settings:\n  allow_requests_on_db_unavailable: {expected}\n");
    let settings = load(&source, &[]);
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Ok(expected)
    );
}

#[rstest]
#[case::padded_upper("  TRUE  ", true)]
#[case::mixed_case_false("False", false)]
#[case::padded_true(" true", true)]
fn trimmed_true_false(#[case] text: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  allow_requests_on_db_unavailable: \"{text}\"\n");
    let settings = load(&source, &[]);
    assert_eq!(
        settings.allow_requests,
        AllowRequestsOnOutage::Text(text.to_owned())
    );
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Ok(expected)
    );
}

#[rstest]
#[case::word("maybe")]
#[case::digit_text("1")]
#[case::empty("")]
#[case::sentence("true, please")]
fn unrecognized_string_false(#[case] text: &str) {
    let source = format!("general_settings:\n  allow_requests_on_db_unavailable: \"{text}\"\n");
    let settings = load(&source, &[]);
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Ok(false)
    );
}

#[rstest]
#[case::number("17")]
#[case::list("[true]")]
#[case::mapping("{enabled: true}")]
fn nonstring_policy_value_fails_at_consumer(#[case] value: &str) {
    let source = format!("general_settings:\n  allow_requests_on_db_unavailable: {value}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.allow_requests, AllowRequestsOnOutage::Unsupported);
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Err(PolicyError::UnsupportedValue)
    );
}

#[rstest]
#[case::true_text("True", true)]
#[case::shared_parser_word("yes", false)]
#[case::empty_text("", false)]
fn environment_reference_uses_string_normalization(#[case] variable: &str, #[case] expected: bool) {
    let settings = load(
        "general_settings:\n  allow_requests_on_db_unavailable: os.environ/ALLOW_ON_DB_OUTAGE\n",
        &[("ALLOW_ON_DB_OUTAGE", variable)],
    );
    assert_eq!(
        allows_requests_during_outage(&settings.allow_requests),
        Ok(expected)
    );
}
