#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::budget_primitives::throttle::{percentage, should_throttle, throttled_limit};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::budget_primitives::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.budget_primitives
}

fn setting_source(value: &str) -> String {
    format!("litellm_settings:\n  budget_exceeded_throttle_percentage: {value}")
}

fn token(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).unwrap() else {
        panic!("token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

#[test]
fn throttle_default_none() {
    let loaded = settings("litellm_settings: {}", &[]);
    assert_eq!(loaded.throttle_pct, None);
    assert_eq!(percentage(loaded.throttle_pct), None);
    assert!(!should_throttle(
        &token(r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#),
        loaded.throttle_pct
    ));
}

#[rstest]
#[case::fraction("0.5", &[], Some(0.5_f64))]
#[case::integer_one("1", &[], Some(1.0_f64))]
#[case::float_one("1.0", &[], Some(1.0_f64))]
#[case::environment_reference(
    "os.environ/THROTTLE_PCT",
    &[("THROTTLE_PCT", "0.25")],
    Some(0.25_f64)
)]
#[case::empty_environment_is_unset("os.environ/THROTTLE_PCT", &[("THROTTLE_PCT", "")], None)]
fn valid_fraction_and_one(
    #[case] value: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<f64>,
) {
    let loaded = settings(&setting_source(value), variables);
    assert_eq!(percentage(loaded.throttle_pct), expected);
}

#[rstest]
#[case::bool_true("true", &[])]
#[case::bool_false("false", &[])]
#[case::zero("0", &[])]
#[case::negative("-0.5", &[])]
#[case::above_one("2.5", &[])]
#[case::quoted_fraction("\"0.5\"", &[])]
#[case::non_numeric_text("abc", &[])]
#[case::overflowing_environment(
    "os.environ/THROTTLE_PCT",
    &[("THROTTLE_PCT", "1e999")]
)]
fn invalid_type_bool_and_nonfinite_disabled(
    #[case] value: &str,
    #[case] variables: &[(&str, &str)],
) {
    let loaded = settings(&setting_source(value), variables);
    assert_eq!(percentage(loaded.throttle_pct), None);
    assert!(!should_throttle(
        &token(r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#),
        loaded.throttle_pct
    ));
    assert_eq!(percentage(Some(f64::NAN)), None);
    assert_eq!(percentage(Some(f64::NEG_INFINITY)), None);
}

#[test]
fn enabled_fraction_still_throttles() {
    let loaded = settings(&setting_source("0.5"), &[]);
    assert_eq!(percentage(loaded.throttle_pct), Some(0.5_f64));
    assert!(should_throttle(
        &token(r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#),
        loaded.throttle_pct
    ));
}

#[test]
fn opted_in_with_zero_limit() {
    let loaded = settings(&setting_source("0.5"), &[]);
    let configured = loaded.throttle_pct;
    let token = token(
        r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":0,"rpm_limit":null}"#,
    );
    assert!(should_throttle(&token, configured));
    assert_eq!(throttled_limit(Some(0), percentage(configured)), Some(1));
}
