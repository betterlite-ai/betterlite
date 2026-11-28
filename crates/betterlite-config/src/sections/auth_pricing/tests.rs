#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_pricing::{MaxBudget, Settings};

fn load(source: &str) -> Result<Settings, ConfigError> {
    load_with(source, &[])
}

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }.load(source)?.sections.auth_pricing)
}

#[rstest]
#[case::empty("{}")]
#[case::other_sections("general_settings:\n  master_key: sk-test\n")]
#[case::explicit_null(
    "general_settings:\n  enforce_user_param: null\n  \
     reject_clientside_metadata_tags: null\nlitellm_settings:\n  max_budget: null\n"
)]
fn pricing_policy_defaults(#[case] source: &str) {
    let defaults = Settings::default();
    assert_eq!(load(source).unwrap(), defaults);
    assert!(!defaults.enforce_user_param);
    assert!(!defaults.reject_clientside_tags);
    assert_eq!(defaults.max_budget, MaxBudget::Finite(Decimal::ZERO));
}

#[rstest]
#[case::enabled("true", true)]
#[case::disabled("false", false)]
#[case::null_defaults("null", false)]
fn enforce_user_loaded(#[case] written: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  enforce_user_param: {written}\n");
    assert_eq!(load(&source).unwrap().enforce_user_param, expected);
}

#[rstest]
#[case::enabled("true", true)]
#[case::disabled("false", false)]
#[case::null_defaults("null", false)]
fn reject_tags_loaded(#[case] written: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  reject_clientside_metadata_tags: {written}\n");
    assert_eq!(load(&source).unwrap().reject_clientside_tags, expected);
}

#[rstest]
#[case::integer("5", MaxBudget::Finite(Decimal::from(5_i64)))]
#[case::float("2.5", MaxBudget::Finite(Decimal::new(25, 1)))]
#[case::zero("0", MaxBudget::Finite(Decimal::ZERO))]
#[case::negative("-1", MaxBudget::Finite(Decimal::from(-1_i64)))]
#[case::numeric_text("\"5\"", MaxBudget::Finite(Decimal::from(5_i64)))]
#[case::infinity("\"inf\"", MaxBudget::PositiveInfinity)]
#[case::negative_infinity("\"-inf\"", MaxBudget::NegativeInfinity)]
#[case::nan("\"NaN\"", MaxBudget::Nan)]
fn proxy_budget_loaded(#[case] written: &str, #[case] expected: MaxBudget) {
    let source = format!("litellm_settings:\n  max_budget: {written}\n");
    assert_eq!(load(&source).unwrap().max_budget, expected);
}

#[test]
fn budget_environment_reference_resolves() {
    let source = "litellm_settings:\n  max_budget: os.environ/PROXY_MAX_BUDGET\n";
    let resolved = load_with(source, &[("PROXY_MAX_BUDGET", "25.5")]).unwrap();
    assert_eq!(resolved.max_budget, MaxBudget::Finite(Decimal::new(255, 1)));
    let unset = load_with(source, &[("PROXY_MAX_BUDGET", "")]).unwrap();
    assert_eq!(unset.max_budget, MaxBudget::Finite(Decimal::ZERO));
}

// an unquoted non-finite float never reaches the section: the YAML layer refuses it for every config key
#[rstest]
#[case::infinity(".inf")]
#[case::signed_infinity("+.inf")]
#[case::negative_infinity("-.inf")]
#[case::nan(".nan")]
#[case::overflow("1e999")]
fn nonfinite_yaml_budget_rejected(#[case] written: &str) {
    let source = format!("litellm_settings:\n  max_budget: {written}\n");
    assert!(matches!(
        load(&source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

#[rstest]
#[case::enforce_text("general_settings:\n  enforce_user_param: enabled\n")]
#[case::enforce_number("general_settings:\n  enforce_user_param: 1\n")]
#[case::tags_text("general_settings:\n  reject_clientside_metadata_tags: enabled\n")]
#[case::budget_list("litellm_settings:\n  max_budget: [1]\n")]
fn invalid_scalar_rejected(#[case] source: &str) {
    assert!(matches!(
        load(source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

// text the float parser cannot read is a load error that names the setting
#[test]
fn budget_text_names_the_field() {
    let error = load("litellm_settings:\n  max_budget: soon\n").unwrap_err();
    assert!(
        error.to_string().contains("litellm_settings.max_budget"),
        "{error}"
    );
}

// a bool is outside the declared float, so the load refuses it and names the setting
#[test]
fn boolean_budget_names_the_field() {
    let error = load("litellm_settings:\n  max_budget: true\n").unwrap_err();
    assert!(
        error.to_string().contains("litellm_settings.max_budget"),
        "{error}"
    );
}

#[test]
fn general_settings_budget_is_unknown() {
    let source = "general_settings:\n  max_budget: 5\n";
    assert!(matches!(
        load(source).unwrap_err(),
        ConfigError::Unsupported { .. }
    ));
}
