#![cfg(test)]
use std::collections::BTreeMap;

use rstest::rstest;

use super::BudgetConfig;
use super::error::BudgetConfigError;
use crate::model_budget_values::cache_keys::BudgetScope;
use crate::model_budget_values::spend_reads::CachedSpend;
use crate::model_budget_values::tests::support::{
    FakeCache, Warnings, amount, budgets, json, runtime,
};
use crate::model_budget_values::usage_report::build_model_max_budget_usage;

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
fn malformed_build_accepts_budget_config_shapes(#[case] raw: &str, #[case] expected: BudgetConfig) {
    assert_eq!(runtime().budget_config(&json(raw)), Ok(expected));
}

#[rstest]
#[case::string_entry(r#""1d""#, BudgetConfigError::NotAnObject)]
#[case::array_entry("[]", BudgetConfigError::NotAnObject)]
#[case::number_entry("5", BudgetConfigError::NotAnObject)]
#[case::null_entry("null", BudgetConfigError::NotAnObject)]
#[case::no_duration(r#"{"max_budget": 1}"#, BudgetConfigError::MissingDuration)]
#[case::null_duration(
    r#"{"max_budget": 1, "budget_duration": null}"#,
    BudgetConfigError::MissingDuration
)]
#[case::null_reported_duration_wins(
    r#"{"budget_duration": "1d", "time_period": null}"#,
    BudgetConfigError::MissingDuration
)]
#[case::number_duration(r#"{"budget_duration": 5}"#, BudgetConfigError::MissingDuration)]
#[case::unparsable_duration(
    r#"{"budget_duration": "banana"}"#,
    BudgetConfigError::UnparsableDuration { spec: "banana".to_owned() }
)]
#[case::empty_duration(
    r#"{"budget_duration": ""}"#,
    BudgetConfigError::UnparsableDuration { spec: String::new() }
)]
#[case::string_budget(
    r#"{"max_budget": "10", "budget_duration": "1d"}"#,
    BudgetConfigError::MaxBudget
)]
#[case::bool_budget(
    r#"{"max_budget": true, "budget_duration": "1d"}"#,
    BudgetConfigError::MaxBudget
)]
#[case::reported_budget_checked(
    r#"{"max_budget": 1, "budget_limit": "x", "budget_duration": "1d"}"#,
    BudgetConfigError::MaxBudget
)]
#[case::fractional_tpm(
    r#"{"budget_duration": "1d", "tpm_limit": 1.5}"#,
    BudgetConfigError::Limit { field: "tpm_limit" }
)]
#[case::string_rpm(
    r#"{"budget_duration": "1d", "rpm_limit": "5"}"#,
    BudgetConfigError::Limit { field: "rpm_limit" }
)]
fn malformed_build_rejects_unusable_entries(
    #[case] raw: &str,
    #[case] expected: BudgetConfigError,
) {
    assert_eq!(runtime().budget_config(&json(raw)), Err(expected));
}

#[tokio::test]
async fn malformed_build_drops_a_bad_entry_from_the_report_without_a_warning() {
    let warnings = Warnings::capture();
    let configured = budgets(
        r#"{
            "good": {"max_budget": 2, "budget_duration": "1d"},
            "no-duration": {"max_budget": 2},
            "bad-duration": {"max_budget": 2, "budget_duration": "soon"},
            "not-an-object": "x",
            "null-entry": null
        }"#,
    );
    let cache = FakeCache::new(None, Some(vec![Some(CachedSpend::Number(amount(1, 0)))]));
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report.keys().collect::<Vec<_>>(), ["good"]);
    assert_eq!(
        cache.memory_reads.lock().unwrap().clone(),
        [["virtual_key_spend:k1:good:1d"]]
    );
    assert_eq!(warnings.count(), 0);
}

#[tokio::test]
async fn malformed_build_reports_nothing_and_reads_nothing_when_every_entry_is_bad() {
    let configured = budgets(r#"{"a": {"max_budget": 1}, "b": 3}"#);
    let cache = FakeCache::new(None, None);
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report, BTreeMap::new());
    assert!(cache.memory_reads.lock().unwrap().is_empty());
    assert!(cache.redis_reads.lock().unwrap().is_empty());
}
