#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::passthrough_request_policies::Settings;

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.passthrough_request_policies)
}

#[test]
fn loads_the_request_timeout() {
    let settings = load(
        "general_settings:\n  pass_through_request_timeout: 45.5\n",
        &[],
    )
    .unwrap();
    assert_eq!(settings.request_timeout.to_bits(), 45.5_f64.to_bits());
}

#[rstest]
#[case::absent("general_settings: {}\n", 600.0_f64)]
#[case::empty_document("{}\n", 600.0_f64)]
#[case::explicit_null("general_settings:\n  pass_through_request_timeout: ~\n", 600.0_f64)]
#[case::integer("general_settings:\n  pass_through_request_timeout: 30\n", 30.0_f64)]
#[case::fraction(
    "general_settings:\n  pass_through_request_timeout: 12.25\n",
    12.25_f64
)]
#[case::zero("general_settings:\n  pass_through_request_timeout: 0\n", 0.0_f64)]
#[case::negative("general_settings:\n  pass_through_request_timeout: -5\n", -5.0_f64)]
#[case::quoted_number(
    "general_settings:\n  pass_through_request_timeout: \"30\"\n",
    30.0_f64
)]
fn resolves_values_and_absence(#[case] source: &str, #[case] expected: f64) {
    assert_eq!(
        load(source, &[]).unwrap().request_timeout.to_bits(),
        expected.to_bits()
    );
}

#[rstest]
#[case::environment("40.5", 40.5_f64)]
#[case::empty_environment("", 600.0_f64)]
fn resolves_an_environment_reference(#[case] variable: &str, #[case] expected: f64) {
    let source =
        "general_settings:\n  pass_through_request_timeout: os.environ/PASS_THROUGH_TIMEOUT\n";
    let settings = load(source, &[("PASS_THROUGH_TIMEOUT", variable)]).unwrap();
    assert_eq!(settings.request_timeout.to_bits(), expected.to_bits());
}

#[rstest]
#[case::boolean("general_settings:\n  pass_through_request_timeout: true\n")]
#[case::not_a_number("general_settings:\n  pass_through_request_timeout: soon\n")]
fn refuses_a_value_that_is_not_a_number(#[case] source: &str) {
    drop(load(source, &[]).unwrap_err());
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/passthrough-valid.yaml");
    assert_eq!(
        load(source, &[]).unwrap().request_timeout.to_bits(),
        45.5_f64.to_bits()
    );
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/passthrough-invalid.yaml");
    drop(load(source, &[]).unwrap_err());
}

#[test]
fn the_default_settings_use_six_hundred_seconds() {
    assert_eq!(
        Settings::default().request_timeout.to_bits(),
        600.0_f64.to_bits()
    );
}
