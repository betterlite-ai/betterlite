#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_pricing::{MaxBudget, Settings};

fn load(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)?
    .sections
    .auth_pricing)
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
#[case::infinity(".inf", MaxBudget::PositiveInfinity)]
#[case::signed_infinity("+.inf", MaxBudget::PositiveInfinity)]
#[case::negative_infinity("-.inf", MaxBudget::NegativeInfinity)]
#[case::nan(".nan", MaxBudget::Nan)]
fn proxy_budget_loaded(#[case] written: &str, #[case] expected: MaxBudget) {
    let source = format!("litellm_settings:\n  max_budget: {written}\n");
    assert_eq!(load(&source).unwrap().max_budget, expected);
}

#[rstest]
#[case::enforce_text("general_settings:\n  enforce_user_param: enabled\n")]
#[case::enforce_number("general_settings:\n  enforce_user_param: 1\n")]
#[case::tags_text("general_settings:\n  reject_clientside_metadata_tags: enabled\n")]
#[case::budget_text("litellm_settings:\n  max_budget: \"5\"\n")]
#[case::budget_bool("litellm_settings:\n  max_budget: true\n")]
#[case::budget_overflow("litellm_settings:\n  max_budget: 1e999\n")]
fn invalid_scalar_rejected(#[case] source: &str) {
    assert!(matches!(
        load(source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

#[test]
fn general_settings_budget_is_unknown() {
    let source = "general_settings:\n  max_budget: 5\n";
    assert!(matches!(
        load(source).unwrap_err(),
        ConfigError::Unsupported { .. }
    ));
}
