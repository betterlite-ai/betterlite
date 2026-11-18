#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::capture_rate::settings::{
    SpendCaptureProvider, SpendCaptureRateCheckSettings,
};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{CaptureRateSettingsError, decode_capture_rate_settings, load_capture_rate_settings};
use crate::loader::ConfigError;

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(
    source: &str,
    env: &BTreeMap<String, String>,
) -> Result<Option<SpendCaptureRateCheckSettings>, CaptureRateSettingsError> {
    load_capture_rate_settings(source, env)
}

fn settings(source: &str) -> SpendCaptureRateCheckSettings {
    let env = environment(&[]);
    load(source, &env).unwrap().unwrap()
}

const PRESENT: &str = "general_settings:\n  spend_capture_rate_check:\n";

#[test]
fn loads_capture_rate_settings_defaults() {
    let defaults = settings(&format!("{PRESENT}    {{}}\n"));
    assert_eq!(defaults.providers, vec![SpendCaptureProvider::Openai]);
    assert_eq!(defaults.threshold, dec("0.9"));
    assert_eq!(defaults.lookback_days, 7);
    assert!(defaults.openai_project_ids.is_empty());

    let partial = settings(&format!("{PRESENT}    threshold: 0.75\n"));
    assert_eq!(partial.threshold, dec("0.75"));
    assert_eq!(partial.providers, vec![SpendCaptureProvider::Openai]);
    assert_eq!(partial.lookback_days, 7);

    let nulled = settings(
        "general_settings:\n  spend_capture_rate_check:\n    providers: null\n    threshold: null\n    lookback_days: null\n    openai_project_ids: null\n",
    );
    assert_eq!(nulled.providers, vec![SpendCaptureProvider::Openai]);
    assert_eq!(nulled.threshold, dec("0.9"));
    assert_eq!(nulled.lookback_days, 7);
    assert!(nulled.openai_project_ids.is_empty());
}

#[rstest]
#[case::empty_document("")]
#[case::other_general_settings("general_settings:\n  master_key: sk-1\n")]
#[case::null_check("general_settings:\n  spend_capture_rate_check: null\n")]
#[case::empty_general_settings("general_settings: {}\n")]
fn absent_or_null_capture_rate_settings_are_disabled(#[case] source: &str) {
    let env = environment(&[]);
    assert_eq!(load(source, &env).unwrap(), None);
}

#[test]
fn retains_duplicate_providers_and_explicit_values() {
    let loaded = settings(&format!(
        "{PRESENT}    providers: [openai, openai]\n    threshold: 0.5\n    lookback_days: 180\n    openai_project_ids: [proj_a, proj_b]\n"
    ));
    assert_eq!(
        loaded.providers,
        vec![SpendCaptureProvider::Openai, SpendCaptureProvider::Openai]
    );
    assert_eq!(loaded.threshold, dec("0.5"));
    assert_eq!(loaded.lookback_days, 180);
    assert_eq!(loaded.openai_project_ids, vec!["proj_a", "proj_b"]);
}

#[test]
fn reads_quoted_numbers_and_integral_floats_like_the_source_model() {
    let loaded = settings(&format!(
        "{PRESENT}    threshold: \"0.75\"\n    lookback_days: 7.0\n"
    ));
    assert_eq!(loaded.threshold, dec("0.75"));
    assert_eq!(loaded.lookback_days, 7);
}

#[test]
fn rejects_an_unknown_capture_rate_setting() {
    let env = environment(&[]);
    let error = load(&format!("{PRESENT}    provider: openai\n"), &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::Yaml { .. })
    ));
}

#[test]
fn rejects_an_unknown_provider() {
    let env = environment(&[]);
    let error = load(&format!("{PRESENT}    providers: [azure]\n"), &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::Yaml { .. })
    ));
}

#[test]
fn rejects_an_empty_provider_list() {
    let env = environment(&[]);
    let error = load(&format!("{PRESENT}    providers: []\n"), &env).unwrap_err();
    assert!(matches!(error, CaptureRateSettingsError::EmptyProviders));
}

#[rstest]
#[case::zero("0")]
#[case::negative("-0.1")]
#[case::above_one("1.0001")]
fn rejects_a_threshold_outside_its_range(#[case] threshold: &str) {
    let env = environment(&[]);
    let source = format!("{PRESENT}    threshold: {threshold}\n");
    let error = load(&source, &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::ThresholdOutOfRange { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("spend_capture_rate_check.threshold")
    );
}

#[rstest]
#[case::smallest("0.0001")]
#[case::whole("1")]
#[case::lower_half("0.5")]
fn accepts_a_threshold_inside_its_range(#[case] threshold: &str) {
    let loaded = settings(&format!("{PRESENT}    threshold: {threshold}\n"));
    assert_eq!(loaded.threshold, dec(threshold));
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
#[case::above_the_maximum(181)]
fn rejects_a_lookback_outside_its_range(#[case] days: i64) {
    let env = environment(&[]);
    let source = format!("{PRESENT}    lookback_days: {days}\n");
    let error = load(&source, &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::LookbackOutOfRange { lookback_days } if lookback_days == days
    ));
    assert!(
        error
            .to_string()
            .contains("spend_capture_rate_check.lookback_days")
    );
}

#[rstest]
#[case::first(1)]
#[case::last(180)]
fn accepts_a_lookback_inside_its_range(#[case] days: i64) {
    let loaded = settings(&format!("{PRESENT}    lookback_days: {days}\n"));
    assert_eq!(loaded.lookback_days, days);
}

#[test]
fn rejects_a_fractional_lookback() {
    let env = environment(&[]);
    let error = load(&format!("{PRESENT}    lookback_days: 7.5\n"), &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::InvalidInteger { .. })
    ));
}

#[test]
fn rejects_a_text_threshold() {
    let env = environment(&[]);
    let error = load(&format!("{PRESENT}    threshold: high\n"), &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::InvalidScalar { .. })
    ));
}

#[test]
fn resolves_environment_references_for_numbers() {
    let env = environment(&[
        ("SPEND_CAPTURE_THRESHOLD", "0.8"),
        ("SPEND_CAPTURE_LOOKBACK", " 14 "),
    ]);
    let loaded = load(
        &format!(
            "{PRESENT}    threshold: os.environ/SPEND_CAPTURE_THRESHOLD\n    lookback_days: os.environ/SPEND_CAPTURE_LOOKBACK\n"
        ),
        &env,
    )
    .unwrap()
    .unwrap();
    assert_eq!(loaded.threshold, dec("0.8"));
    assert_eq!(loaded.lookback_days, 14);
}

#[test]
fn an_empty_environment_value_keeps_the_default() {
    let env = environment(&[
        ("SPEND_CAPTURE_THRESHOLD", ""),
        ("SPEND_CAPTURE_LOOKBACK", "   "),
    ]);
    let loaded = load(
        &format!(
            "{PRESENT}    threshold: os.environ/SPEND_CAPTURE_THRESHOLD\n    lookback_days: os.environ/SPEND_CAPTURE_LOOKBACK\n"
        ),
        &env,
    )
    .unwrap()
    .unwrap();
    assert_eq!(loaded.threshold, dec("0.9"));
    assert_eq!(loaded.lookback_days, 7);
}

#[test]
fn a_missing_environment_reference_is_a_load_error() {
    let env = environment(&[]);
    let error = load(
        &format!("{PRESENT}    threshold: os.environ/SPEND_CAPTURE_THRESHOLD\n"),
        &env,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::MissingEnvironment { variable, .. })
            if variable == "SPEND_CAPTURE_THRESHOLD"
    ));
}

#[test]
fn an_unreadable_environment_number_is_a_load_error() {
    let env = environment(&[("SPEND_CAPTURE_LOOKBACK", "many")]);
    let error = load(
        &format!("{PRESENT}    lookback_days: os.environ/SPEND_CAPTURE_LOOKBACK\n"),
        &env,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::Environment { .. })
    ));
}

#[test]
fn stays_disabled_without_a_general_settings_mapping() {
    let env = environment(&[]);
    let error = load("general_settings: [openai]\n", &env).unwrap_err();
    assert!(matches!(
        error,
        CaptureRateSettingsError::Config(ConfigError::Yaml { .. })
    ));
}

#[test]
fn decodes_a_check_without_an_environment() {
    let source = format!("{PRESENT}    lookback_days: 3\n");
    let decoded = decode_capture_rate_settings(&source).unwrap().unwrap();
    assert_eq!(decoded.providers, vec![SpendCaptureProvider::Openai]);
    assert_eq!(decoded.lookback_days, 3);
    assert!(decode_capture_rate_settings("{}").unwrap().is_none());
}

#[rstest]
fn replays_the_capture_rate_settings_corpus(
    #[files("fuzz/corpus/capture_rate_settings/*")] path: PathBuf,
) {
    let text = fs::read_to_string(path).unwrap();
    let _decoded = decode_capture_rate_settings(&text);
}
