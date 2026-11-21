#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::prometheus_exporter::Settings;

const BUCKETS: &str = "litellm_settings:\n  prometheus_latency_buckets: ";
const FLAG: &str = "litellm_settings:\n  prometheus_emit_input_sequence_length_label: ";
const BUCKETS_PATH: &str = "litellm_settings.prometheus_latency_buckets";

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.prometheus_exporter)
}

fn load(source: &str) -> Settings {
    load_with(source, &[]).unwrap()
}

#[test]
fn keeps_the_defaults_when_no_key_is_set() {
    let settings = load("model_list: []\n");
    assert_eq!(settings.prometheus_latency_buckets, None);
    assert!(!settings.prometheus_emit_input_sequence_length_label);
    assert_eq!(settings, Settings::default());
}

#[test]
fn reads_a_configured_bucket_list() {
    let settings = load(&format!("{BUCKETS}[0.1, 0.5, 1.0]\n"));
    assert_eq!(
        settings.prometheus_latency_buckets,
        Some(vec![0.1_f64, 0.5_f64, 1.0_f64])
    );
}

#[test]
fn reads_integer_buckets_as_seconds() {
    let settings = load(&format!("{BUCKETS}[1, 2]\n"));
    assert_eq!(
        settings.prometheus_latency_buckets,
        Some(vec![1.0_f64, 2.0_f64])
    );
}

#[test]
fn keeps_an_empty_bucket_list_as_set() {
    assert_eq!(
        load(&format!("{BUCKETS}[]\n")).prometheus_latency_buckets,
        Some(Vec::new())
    );
}

#[test]
fn treats_a_null_bucket_list_as_unset() {
    assert_eq!(
        load(&format!("{BUCKETS}null\n")).prometheus_latency_buckets,
        None
    );
}

#[test]
fn reads_a_bucket_from_an_environment_reference() {
    let settings = load_with(
        &format!("{BUCKETS}[os.environ/FIRST_BUCKET, 2]\n"),
        &[("FIRST_BUCKET", "0.25")],
    )
    .unwrap();
    assert_eq!(
        settings.prometheus_latency_buckets,
        Some(vec![0.25_f64, 2.0_f64])
    );
}

#[test]
fn reports_a_missing_environment_variable_for_a_bucket() {
    let error = load_with(&format!("{BUCKETS}[os.environ/FIRST_BUCKET]\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "FIRST_BUCKET" && path == BUCKETS_PATH
    ));
}

#[rstest]
#[case::scalar("0.5")]
#[case::object("{second: 1}")]
#[case::booleans("[true, false]")]
fn rejects_a_bucket_list_not_made_of_numbers(#[case] value: &str) {
    let error = load_with(&format!("{BUCKETS}{value}\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == BUCKETS_PATH
    ));
}

#[test]
fn rejects_a_bucket_that_is_not_a_number() {
    let error = load_with(&format!("{BUCKETS}[0.5, soon]\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidFloat { path, .. } if path == BUCKETS_PATH
    ));
}

#[test]
fn enables_the_input_sequence_length_label_only_for_true() {
    assert!(load(&format!("{FLAG}true\n")).prometheus_emit_input_sequence_length_label);
    assert!(!load(&format!("{FLAG}false\n")).prometheus_emit_input_sequence_length_label);
}

#[rstest]
#[case::text("'true'")]
#[case::number("1")]
#[case::null("null")]
fn keeps_the_input_sequence_length_label_off_for_anything_but_true(#[case] value: &str) {
    assert!(!load(&format!("{FLAG}{value}\n")).prometheus_emit_input_sequence_length_label);
}

#[test]
fn keeps_the_bucket_default_when_only_the_flag_is_set() {
    let settings = load(&format!("{FLAG}true\n"));
    assert_eq!(settings.prometheus_latency_buckets, None);
    assert!(settings.prometheus_emit_input_sequence_length_label);
}
