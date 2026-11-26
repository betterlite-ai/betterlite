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

#[test]
fn max_budget_default_zero() {
    let absent = load("litellm_settings: {}", &[]).expect("an empty section loads");
    assert_eq!(absent.max_budget, Decimal::ZERO);
    assert!(!absent.global_spend_enabled());
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
    assert!(loaded.global_spend_enabled());
    let literal =
        load("litellm_settings:\n  max_budget: 100.0", &[]).expect("a literal ceiling loads");
    assert_eq!(literal.max_budget, amount("100.0"));
    assert!(literal.global_spend_enabled());
}

// get_global_proxy_spend gates on `litellm.max_budget > 0`, so these values never read the aggregate
#[rstest]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::negative("-5")]
#[case::nan(".nan")]
fn zero_negative_and_nan_skip_read(#[case] value: &str) {
    let source = format!("litellm_settings:\n  max_budget: {value}");
    let loaded = load(&source, &[]).expect("a non-positive ceiling still loads");
    assert_eq!(loaded.max_budget, Decimal::ZERO, "{value}");
    assert!(!loaded.global_spend_enabled(), "{value}");
}

#[rstest]
#[case::text("not-a-number")]
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
