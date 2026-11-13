#![cfg(test)]
use std::collections::BTreeMap;

use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::spend_reads::CachedSpend;
use crate::model_budget_values::tests::support::{FakeCache, Warnings, amount, budgets, runtime};
use crate::model_budget_values::usage_report::build_model_max_budget_usage;

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
