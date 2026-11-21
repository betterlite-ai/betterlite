#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::ConfigError;
use crate::loader::ConfigLoader;
use crate::sections::session_iteration_limits::DEFAULT_TTL_SECONDS;
use crate::sections::session_iteration_limits::Settings;
use crate::sections::session_iteration_limits::TTL_VARIABLE;

#[test]
fn defaults_to_3600_seconds() {
    let settings = settings(&[]).expect("the defaults load");
    assert_eq!(
        settings.max_iterations_ttl,
        Duration::from_secs(DEFAULT_TTL_SECONDS)
    );
}

#[test]
fn environment_value_overrides_the_default() {
    let settings = settings(&[(TTL_VARIABLE, "120")]).expect("the override loads");
    assert_eq!(settings.max_iterations_ttl, Duration::from_secs(120));
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
fn empty_value_reads_as_unset(#[case] value: &str) {
    let settings = settings(&[(TTL_VARIABLE, value)]).expect("an empty value is unset");
    assert_eq!(
        settings.max_iterations_ttl,
        Duration::from_secs(DEFAULT_TTL_SECONDS)
    );
}

#[rstest]
#[case::word("soon")]
#[case::fraction("1.5")]
#[case::negative("-5")]
fn invalid_value_stops_the_load_and_names_the_variable(#[case] value: &str) {
    let error =
        settings(&[(TTL_VARIABLE, value)]).expect_err("a value that is not a count stops the load");
    assert!(error.to_string().contains(TTL_VARIABLE), "{error}");
}

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.session_iteration_limits)
}
