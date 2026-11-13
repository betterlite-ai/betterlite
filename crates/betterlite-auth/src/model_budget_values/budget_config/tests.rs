#![cfg(test)]
use rstest::rstest;

use super::BudgetConfig;
use super::error::BudgetConfigError;
use crate::model_budget_values::tests::support::{amount, json, runtime};

fn config(
    max_budget: Option<rust_decimal::Decimal>,
    budget_duration: &str,
    tpm_limit: Option<i64>,
    rpm_limit: Option<i64>,
) -> BudgetConfig {
    BudgetConfig {
        max_budget,
        budget_duration: budget_duration.to_owned(),
        tpm_limit,
        rpm_limit,
    }
}

#[rstest]
#[case::minimal(
    r#"{"max_budget": 10, "budget_duration": "1d"}"#,
    config(Some(amount(10, 0)), "1d", None, None)
)]
#[case::fraction(
    r#"{"max_budget": 0.5, "budget_duration": "1h"}"#,
    config(Some(amount(5, 1)), "1h", None, None)
)]
#[case::exponent(
    r#"{"max_budget": 1e-5, "budget_duration": "1h"}"#,
    config(Some(amount(1, 5)), "1h", None, None)
)]
#[case::zero_budget(
    r#"{"max_budget": 0, "budget_duration": "1h"}"#,
    config(Some(amount(0, 0)), "1h", None, None)
)]
#[case::no_budget(
    r#"{"budget_duration": "1h", "tpm_limit": 5}"#,
    config(None, "1h", Some(5), None)
)]
#[case::null_budget(
    r#"{"max_budget": null, "budget_duration": "1h"}"#,
    config(None, "1h", None, None)
)]
#[case::limits(
    r#"{"budget_duration": "1h", "tpm_limit": 100, "rpm_limit": 7}"#,
    config(None, "1h", Some(100), Some(7))
)]
#[case::null_limits(
    r#"{"budget_duration": "1h", "tpm_limit": null, "rpm_limit": null}"#,
    config(None, "1h", None, None)
)]
#[case::reported_names(
    r#"{"budget_limit": 5, "time_period": "30s"}"#,
    config(Some(amount(5, 0)), "30s", None, None)
)]
#[case::reported_names_win(
    r#"{"max_budget": 1, "budget_limit": 2, "budget_duration": "1d", "time_period": "1h"}"#,
    config(Some(amount(2, 0)), "1h", None, None)
)]
#[case::null_reported_budget_wins(
    r#"{"max_budget": 1, "budget_limit": null, "budget_duration": "1d"}"#,
    config(None, "1d", None, None)
)]
#[case::unknown_keys_ignored(
    r#"{"budget_duration": "1d", "color": "red"}"#,
    config(None, "1d", None, None)
)]
fn budget_config_accepts_configured_shapes(#[case] raw: &str, #[case] expected: BudgetConfig) {
    assert_eq!(runtime().budget_config(&json(raw)).unwrap(), expected);
}

fn is_entry(error: &BudgetConfigError) -> bool {
    matches!(error, BudgetConfigError::Entry { .. })
}

fn is_not_object(error: &BudgetConfigError) -> bool {
    matches!(error, BudgetConfigError::NotObject)
}

fn is_missing_duration(error: &BudgetConfigError) -> bool {
    matches!(error, BudgetConfigError::MissingDuration)
}

fn is_unparsable_duration(error: &BudgetConfigError) -> bool {
    matches!(error, BudgetConfigError::UnparsableDuration { .. })
}

fn is_max_budget(error: &BudgetConfigError) -> bool {
    matches!(error, BudgetConfigError::MaxBudget)
}

#[rstest]
#[case::string_entry(r#""1d""#, is_not_object)]
#[case::array_entry("[]", is_not_object)]
#[case::object_array_entry("[{}]", is_not_object)]
#[case::number_entry("5", is_not_object)]
#[case::null_entry("null", is_not_object)]
#[case::no_duration(r#"{"max_budget": 1}"#, is_missing_duration)]
#[case::null_duration(r#"{"max_budget": 1, "budget_duration": null}"#, is_missing_duration)]
#[case::null_reported_duration_wins(
    r#"{"budget_duration": "1d", "time_period": null}"#,
    is_missing_duration
)]
#[case::number_duration(r#"{"budget_duration": 5}"#, is_entry)]
#[case::unparsable_duration(r#"{"budget_duration": "banana"}"#, is_unparsable_duration)]
#[case::empty_duration(r#"{"budget_duration": ""}"#, is_unparsable_duration)]
#[case::string_budget(r#"{"max_budget": "10", "budget_duration": "1d"}"#, is_entry)]
#[case::bool_budget(r#"{"max_budget": true, "budget_duration": "1d"}"#, is_entry)]
#[case::reported_budget_checked(
    r#"{"max_budget": 1, "budget_limit": "x", "budget_duration": "1d"}"#,
    is_entry
)]
#[case::unrepresentable_budget(r#"{"max_budget": 1e30, "budget_duration": "1d"}"#, is_max_budget)]
#[case::fractional_tpm(r#"{"budget_duration": "1d", "tpm_limit": 1.5}"#, is_entry)]
#[case::string_rpm(r#"{"budget_duration": "1d", "rpm_limit": "5"}"#, is_entry)]
fn budget_config_rejects_unusable_entries(
    #[case] raw: &str,
    #[case] expected: fn(&BudgetConfigError) -> bool,
) {
    let error = runtime().budget_config(&json(raw)).unwrap_err();
    assert!(expected(&error));
}

#[test]
fn budget_config_reports_the_duration_it_cannot_parse() {
    let error = runtime()
        .budget_config(&json(r#"{"budget_duration": "banana"}"#))
        .unwrap_err();
    assert!(matches!(
        error,
        BudgetConfigError::UnparsableDuration { spec } if spec == "banana"
    ));
}
