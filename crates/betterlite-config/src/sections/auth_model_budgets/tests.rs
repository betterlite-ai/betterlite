#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_model_budgets::Settings;

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .auth_model_budgets)
}

fn amount(text: &str) -> Decimal {
    text.parse().unwrap()
}

// the spend read this value gates is betterlite-auth's loaded_budget_* tests, which consume it through the loader
#[test]
fn max_budget_default_zero() {
    let absent = load("litellm_settings: {}", &[]).expect("an empty section loads");
    assert_eq!(absent.max_budget, Decimal::ZERO);
    let unset = load("model_list: []", &[]).expect("a config without the key loads");
    assert_eq!(unset, Settings::default());
}

#[test]
fn positive_budget_enables_read() {
    let source = "\
litellm_settings:
  max_budget: os.environ/PROXY_MAX_BUDGET";
    let loaded = load(source, &[("PROXY_MAX_BUDGET", "250.50")])
        .expect("the environment resolves the ceiling");
    assert_eq!(loaded.max_budget, amount("250.50"));
    assert!(loaded.max_budget > Decimal::ZERO);
    let literal =
        load("litellm_settings:\n  max_budget: 100.0", &[]).expect("a literal ceiling loads");
    assert_eq!(literal.max_budget, amount("100.0"));
    assert!(literal.max_budget > Decimal::ZERO);
}

// the spend read gates on `max_budget > 0`: a quoted NaN reads as nan and skips it, and an unquoted .nan never reaches this section
#[rstest]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::negative("-5")]
#[case::nan("\"NaN\"")]
fn zero_negative_and_nan_skip_read(#[case] value: &str) {
    let source = format!("litellm_settings:\n  max_budget: {value}");
    let loaded = load(&source, &[]).expect("a non-positive ceiling still loads");
    let expected = if value == "-5" {
        Decimal::new(-5, 0)
    } else {
        Decimal::ZERO
    };
    assert_eq!(loaded.max_budget, expected, "{value}");
    assert!(loaded.max_budget <= Decimal::ZERO, "{value}");
}

// the shared YAML layer refuses a non-finite literal, so it never becomes a ceiling
#[test]
fn nonfinite_literal_fails_at_the_yaml_layer() {
    let error = load("litellm_settings:\n  max_budget: .nan", &[])
        .expect_err("a non-finite YAML budget fails the load");
    assert_eq!(error.category(), "yaml");
}

#[rstest]
#[case::text("not-a-number")]
#[case::boolean("true")]
#[case::out_of_range("1.0e29")]
fn invalid_budget_rejected(#[case] value: &str) {
    let source = format!("litellm_settings:\n  max_budget: {value}");
    let error = load(&source, &[]).expect_err("a budget that is not money fails the load");
    assert!(
        error.to_string().contains("litellm_settings.max_budget"),
        "{error}"
    );
    assert_eq!(error.category(), "invalid_scalar");
}

#[rstest]
#[case::null("litellm_settings:\n  max_budget: null", vec![])]
#[case::empty_env("litellm_settings:\n  max_budget: os.environ/BUDGET", vec![("BUDGET", "")])]
fn max_budget_null_and_empty_env_default(
    #[case] source: &str,
    #[case] variables: Vec<(&str, &str)>,
) {
    let settings = load(source, &variables).unwrap();
    assert_eq!(settings.max_budget, Decimal::ZERO);
}

#[test]
fn max_budget_missing_env_names_the_field() {
    let error = load("litellm_settings:\n  max_budget: os.environ/BUDGET", &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, path }
        if variable == "BUDGET" && path == "litellm_settings.max_budget")
    );
}
