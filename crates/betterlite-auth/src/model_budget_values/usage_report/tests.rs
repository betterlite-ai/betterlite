#![cfg(test)]
use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{ModelBudgetUsage, build_model_max_budget_usage};
use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::spend_reads::CachedSpend;
use crate::model_budget_values::spend_reads::error::SpendReadError;
use crate::model_budget_values::tests::support::{
    FakeCache, FakeFailure, amount, budgets, runtime,
};

const ONE_BUDGET: &str = r#"{"gpt-4o": {"max_budget": 10, "budget_duration": "1d"}}"#;

fn memory_cache(spends: Vec<Option<CachedSpend>>) -> FakeCache {
    FakeCache::new(None, Some(spends))
}

fn usage(current_spend: Decimal, budget_limit: Decimal, time_period: &str) -> ModelBudgetUsage {
    ModelBudgetUsage {
        current_spend,
        budget_limit: Some(budget_limit),
        time_period: time_period.to_owned(),
    }
}

#[tokio::test]
async fn usage_report_is_empty_without_a_cache() {
    let configured = budgets(ONE_BUDGET);
    let report = build_model_max_budget_usage::<FakeCache>(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        None,
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report, BTreeMap::new());
}

#[tokio::test]
async fn usage_report_is_empty_without_an_entity_and_reads_nothing() {
    let configured = budgets(ONE_BUDGET);
    let cache = memory_cache(vec![None]);
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        None,
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report, BTreeMap::new());
    assert!(cache.redis_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn usage_report_is_empty_without_model_budgets() {
    let cache = memory_cache(vec![None]);
    let report =
        build_model_max_budget_usage(BudgetScope::Key, Some("k1"), None, Some(&cache), runtime())
            .await
            .unwrap();
    assert_eq!(report, BTreeMap::new());
    assert!(cache.redis_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn usage_report_is_empty_for_an_empty_budget_map() {
    let configured = budgets("{}");
    let cache = memory_cache(Vec::new());
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
    assert!(cache.redis_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn usage_report_names_each_budget_by_its_configured_key() {
    let configured = budgets(
        r#"{
            "gpt-4o": {"max_budget": 10, "budget_duration": "1d"},
            "openai/gpt-4": {"max_budget": 2.5, "budget_duration": "1h"}
        }"#,
    );
    let cache = memory_cache(vec![
        Some(CachedSpend::Number(amount(3, 0))),
        Some(CachedSpend::Number(amount(5, 1))),
    ]);
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    let expected = BTreeMap::from([
        (
            "gpt-4o".to_owned(),
            usage(amount(3, 0), amount(10, 0), "1d"),
        ),
        (
            "openai/gpt-4".to_owned(),
            usage(amount(5, 1), amount(25, 1), "1h"),
        ),
    ]);
    assert_eq!(report, expected);
    assert_eq!(
        cache.memory_reads.lock().unwrap().clone(),
        [[
            "virtual_key_spend:k1:gpt-4o:1d",
            "virtual_key_spend:k1:openai/gpt-4:1h"
        ]]
    );
}

#[rstest]
#[case::key(BudgetScope::Key, "virtual_key_spend:e1:gpt-4o:1d")]
#[case::user(BudgetScope::User, "user_model_spend:e1:gpt-4o:1d")]
#[case::end_user(BudgetScope::EndUser, "end_user_model_spend:e1:gpt-4o:1d")]
#[case::team(BudgetScope::Team, "team_model_spend:e1:gpt-4o:1d")]
#[tokio::test]
async fn usage_report_reads_the_spend_key_of_its_scope(
    #[case] scope: BudgetScope,
    #[case] expected: &str,
) {
    let configured = budgets(ONE_BUDGET);
    let cache = memory_cache(vec![None]);
    build_model_max_budget_usage(
        scope,
        Some("e1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(cache.memory_reads.lock().unwrap().clone(), [[expected]]);
}

#[rstest]
#[case::rounds_down(amount(123_449, 6), amount(1234, 4))]
#[case::rounds_up(amount(123_451, 6), amount(1235, 4))]
#[case::keeps_four_places(amount(12_345, 4), amount(12_345, 4))]
#[case::tiny_spend_is_zero(amount(4, 5), Decimal::ZERO)]
#[tokio::test]
async fn usage_report_rounds_spend_to_four_places(
    #[case] spend: Decimal,
    #[case] expected: Decimal,
) {
    let configured = budgets(ONE_BUDGET);
    let cache = memory_cache(vec![Some(CachedSpend::Number(spend))]);
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report["gpt-4o"].current_spend, expected);
}

#[tokio::test]
async fn usage_report_leaves_the_budget_limit_unrounded_and_optional() {
    let configured = budgets(
        r#"{
            "precise": {"max_budget": 1.23456, "budget_duration": "1d"},
            "unlimited": {"budget_duration": "30s", "rpm_limit": 3}
        }"#,
    );
    let cache = memory_cache(vec![None, None]);
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report["precise"].budget_limit, Some(amount(123_456, 5)));
    assert_eq!(report["unlimited"].budget_limit, None);
    assert_eq!(report["unlimited"].time_period, "30s");
}

#[tokio::test]
async fn usage_report_reads_redis_over_memory() {
    let configured = budgets(ONE_BUDGET);
    let held = BTreeMap::from([(
        "virtual_key_spend:k1:gpt-4o:1d".to_owned(),
        CachedSpend::Text("4.5".to_owned()),
    )]);
    let cache = FakeCache::new(
        Some(Ok(held)),
        Some(vec![Some(CachedSpend::Number(amount(99, 0)))]),
    );
    let report = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap();
    assert_eq!(report["gpt-4o"].current_spend, amount(45, 1));
    assert!(cache.memory_reads.lock().unwrap().is_empty());
}

#[test]
fn usage_report_serializes_with_the_litellm_field_names() {
    let usage = ModelBudgetUsage {
        current_spend: amount(15, 1),
        budget_limit: Some(amount(25, 1)),
        time_period: "1d".to_owned(),
    };
    let parsed = serde_json::to_value(&usage).unwrap();
    let mut names: Vec<_> = parsed.as_object().unwrap().keys().cloned().collect();
    names.sort();
    assert_eq!(names, ["budget_limit", "current_spend", "time_period"]);
    assert_eq!(parsed["time_period"], "1d");
    assert!(parsed["current_spend"].is_number());
    assert!(parsed["budget_limit"].is_number());
    assert_eq!(parsed["current_spend"].as_f64(), Some(1.5_f64));
    assert_eq!(parsed["budget_limit"].as_f64(), Some(2.5_f64));

    let unlimited = ModelBudgetUsage {
        current_spend: amount(0, 0),
        budget_limit: None,
        time_period: "1h".to_owned(),
    };
    assert!(serde_json::to_value(&unlimited).unwrap()["budget_limit"].is_null());
}

#[tokio::test]
async fn usage_report_surfaces_a_redis_failure() {
    let configured = budgets(ONE_BUDGET);
    let cache = FakeCache::new(Some(Err(FakeFailure)), None);
    let error = build_model_max_budget_usage(
        BudgetScope::Key,
        Some("k1"),
        Some(&configured),
        Some(&cache),
        runtime(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, SpendReadError::Redis { .. }));
}
