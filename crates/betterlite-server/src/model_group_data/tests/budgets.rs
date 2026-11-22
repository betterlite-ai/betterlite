use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Number;

use super::super::budgets::{
    budget_or_none, budget_response, delete_group_budget_row, model_access_group_budget_row,
    model_access_group_budget_rows, with_budget,
};
use super::super::types::{AccessGroupInfo, ModelAccessGroupBudget};
use super::fixtures::cache::RecordingCache;
use super::fixtures::json_object;
use super::fixtures::scripted::ScriptedDatabase;
use super::{capped_budget, linked_group, names, text, value_error_message};

fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite number"))
}

fn encoded<T: Serialize>(value: &T) -> Json {
    let stored = serde_json::to_value(value).expect("the encoded value");
    serde_json::from_value(stored).expect("the JSON form")
}

#[test]
fn flattened_budget_defaults() {
    let defaults: ModelAccessGroupBudget =
        serde_json::from_str(r#"{"access_group_name": "group-a"}"#).expect("the record");
    assert_eq!(defaults.spend, Decimal::ZERO);
    assert_eq!(defaults.max_budget, None);
    assert_eq!(
        encoded(&defaults),
        Json::Object(json_object(&[
            ("access_group_name", text("group-a")),
            ("spend", number(0.0)),
            ("max_budget", Json::Null),
        ]))
    );
    let explicit: ModelAccessGroupBudget = serde_json::from_str(
        r#"{"access_group_name": "group-b", "spend": 2.5, "max_budget": 10.0}"#,
    )
    .expect("the record");
    assert_eq!(explicit.spend, Decimal::new(25, 1));
    assert_eq!(explicit.max_budget, Some(Decimal::new(10, 0)));
}

#[test]
fn budget_join_missing_vs_present() {
    let present = linked_group(
        "group-a",
        Some(Decimal::new(35, 1)),
        Some("budget-1"),
        Some(capped_budget()),
    );
    let without_row = linked_group("group-b", Some(Decimal::ZERO), None, None);
    let without_join = linked_group("group-c", Some(Decimal::ZERO), Some("budget-gone"), None);
    let projected = budget_or_none(Some(&present)).expect("the joined budget");
    assert_eq!(projected.budget_id, "budget-1");
    assert_eq!(projected.max_budget, Some(Decimal::new(250, 1)));
    assert_eq!(projected.soft_budget, Some(Decimal::new(200, 1)));
    assert_eq!(projected.budget_duration.as_deref(), Some("30d"));
    assert_eq!(
        projected.budget_reset_at,
        Some(Timestamp::Naive(
            "2026-01-01T00:00:00".parse().expect("a naive timestamp"),
        ))
    );
    assert_eq!(budget_or_none(Some(&without_row)), None);
    assert_eq!(budget_or_none(Some(&without_join)), None);
    assert_eq!(budget_or_none(None), None);
}

#[test]
fn with_budget_spend_rules() {
    let info = AccessGroupInfo {
        access_group: "group-a".to_owned(),
        model_names: names(&["alpha", "beta"]),
        deployment_count: 2,
        spend: None,
        budget: None,
    };
    let absent = with_budget(&info, None);
    assert_eq!(absent.spend, Some(Decimal::ZERO));
    assert_eq!(absent.budget, None);
    assert_eq!(absent.model_names, names(&["alpha", "beta"]));
    assert_eq!(absent.deployment_count, 2);
    let no_spend = linked_group("group-a", None, None, None);
    assert_eq!(
        with_budget(&info, Some(&no_spend)).spend,
        Some(Decimal::ZERO)
    );
    let spent = linked_group(
        "group-a",
        Some(Decimal::new(35, 1)),
        Some("budget-1"),
        Some(capped_budget()),
    );
    let listed = with_budget(&info, Some(&spent));
    assert_eq!(listed.spend, Some(Decimal::new(35, 1)));
    assert_eq!(
        listed.budget.expect("the joined budget").budget_id,
        "budget-1"
    );
}

#[test]
fn budget_response_reads_group_and_row() {
    let absent = budget_response("group-a", None);
    assert_eq!(absent.access_group, "group-a");
    assert_eq!(absent.spend, Decimal::ZERO);
    assert_eq!(absent.budget, None);
    let spent = linked_group(
        "group-a",
        Some(Decimal::new(35, 1)),
        Some("budget-1"),
        Some(capped_budget()),
    );
    let found = budget_response("group-a", Some(&spent));
    assert_eq!(found.spend, Decimal::new(35, 1));
    assert_eq!(
        found.budget.expect("the joined budget").max_budget,
        Some(Decimal::new(250, 1))
    );
}

#[tokio::test]
async fn budget_rows_map_by_name_in_one_read() {
    let database = ScriptedDatabase::new(
        Vec::new(),
        vec![capped_budget()],
        vec![
            linked_group(
                "group-a",
                Some(Decimal::new(35, 1)),
                Some("budget-1"),
                Some(capped_budget()),
            ),
            linked_group("group-b", Some(Decimal::ZERO), None, None),
        ],
    );
    let mapped = model_access_group_budget_rows(&database)
        .await
        .expect("the budget rows");
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped["group-a"].spend, Some(Decimal::new(35, 1)));
    assert_eq!(mapped["group-b"].litellm_budget_table, None);
    assert_eq!(database.budget_reads(), 1);
    let one = model_access_group_budget_row("group-a", &database)
        .await
        .expect("the budget row")
        .expect("the seeded group");
    assert_eq!(one.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(one.litellm_budget_table, Some(capped_budget()));
    assert!(
        model_access_group_budget_row("missing", &database)
            .await
            .expect("the budget row")
            .is_none()
    );
}

#[tokio::test]
async fn budget_delete_retains_shared_row() {
    let database = ScriptedDatabase::new(
        Vec::new(),
        vec![capped_budget()],
        vec![linked_group(
            "group-a",
            Some(Decimal::new(35, 1)),
            Some("budget-1"),
            Some(capped_budget()),
        )],
    );
    let cache = RecordingCache::default();
    let deleted = delete_group_budget_row("group-a", &database, &cache)
        .await
        .expect("the delete");
    assert!(deleted);
    assert!(database.budget("group-a").is_none());
    assert!(database.has_budget_row("budget-1"));
    assert_eq!(
        cache.evictions(),
        vec![names(&[
            "model_access_group:group-a",
            "model_access_group_registry",
        ])]
    );
}

#[tokio::test]
async fn absent_row_still_evicts() {
    let database = ScriptedDatabase::new(Vec::new(), Vec::new(), Vec::new());
    let cache = RecordingCache::default();
    let deleted = delete_group_budget_row("group-a", &database, &cache)
        .await
        .expect("the delete");
    assert!(!deleted);
    assert_eq!(database.delete_calls(), names(&["group-a"]));
    assert_eq!(
        cache.evictions(),
        vec![names(&[
            "model_access_group:group-a",
            "model_access_group_registry",
        ])]
    );
}

#[tokio::test]
async fn budget_delete_failure_order() {
    let mut database = ScriptedDatabase::new(Vec::new(), Vec::new(), Vec::new());
    database.fail_delete = true;
    let cache = RecordingCache::default();
    let error = delete_group_budget_row("group-a", &database, &cache)
        .await
        .expect_err("the delete failure");
    assert_eq!(value_error_message(error), "scripted delete failure");
    assert_eq!(database.delete_calls(), names(&["group-a"]));
    assert!(cache.evictions().is_empty());
}
