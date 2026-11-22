#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(yaml: &str) -> Result<bool, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(yaml)
    .map(|config| config.sections.mock_testing_params.allowed)
}

#[rstest]
#[case::absent("{}", false)]
#[case::null(
    "general_settings: {dangerously_allow_mock_testing_request_params: null}",
    false
)]
#[case::enabled(
    "general_settings: {dangerously_allow_mock_testing_request_params: true}",
    true
)]
#[case::disabled(
    "general_settings: {dangerously_allow_mock_testing_request_params: false}",
    false
)]
fn mock_testing_opt_in_loads(#[case] yaml: &str, #[case] expected: bool) {
    assert_eq!(load(yaml).expect("the config loads"), expected);
}

#[rstest]
#[case::text("general_settings: {dangerously_allow_mock_testing_request_params: 'true'}")]
#[case::number("general_settings: {dangerously_allow_mock_testing_request_params: 1}")]
#[case::list("general_settings: {dangerously_allow_mock_testing_request_params: [true]}")]
#[case::object("general_settings: {dangerously_allow_mock_testing_request_params: {}}")]
fn mock_testing_opt_in_refuses_wrong_types(#[case] yaml: &str) {
    let error = load(yaml).expect_err("a value of the wrong type fails the load");
    let ConfigError::InvalidScalar { path, .. } = error else {
        panic!("a wrong type is not a field error");
    };
    assert_eq!(
        path,
        "general_settings.dangerously_allow_mock_testing_request_params"
    );
}
