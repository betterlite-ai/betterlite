use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{FakeRows, Fixture, auth, dec, group_budget, group_row};

#[tokio::test]
async fn group_matched_uses_only_the_token_groups() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","matched_model_access_groups":["a","b"]}"#);
    fixture.rows = FakeRows::new().with_groups(vec![
        group_row("a", "1", r#"{"max_budget":5}"#),
        group_row("c", "1", r#"{"max_budget":9}"#),
    ]);
    let list = fixture.counters().await;
    let names: Vec<&str> = list
        .iter()
        .map(|counter| counter.entity_id.as_str())
        .collect();
    assert_eq!(names, vec!["a"]);
    assert_eq!(
        fixture.rows.group_queries(),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
}

#[tokio::test]
async fn group_batch_merges_the_cached_and_fetched_budgets() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","matched_model_access_groups":["a","b"]}"#);
    fixture
        .cache
        .put("model_access_group:a", &group_budget("a", "3", "5"));
    fixture.rows = FakeRows::new().with_groups(vec![group_row("b", "2", r#"{"max_budget":7}"#)]);
    let list = fixture.counters().await;
    let caps: Vec<Decimal> = list.iter().map(|counter| counter.max_budget).collect();
    assert_eq!(caps, vec![dec("5"), dec("7")]);
    let spends: Vec<Decimal> = list.iter().map(|counter| counter.fallback_spend).collect();
    assert_eq!(spends, vec![dec("3"), dec("2")]);
    assert_eq!(fixture.rows.group_queries(), vec![vec!["b".to_owned()]]);
    let keys: Vec<&str> = list
        .iter()
        .map(|counter| counter.counter_key.as_str())
        .collect();
    assert_eq!(
        keys,
        vec!["spend:model_access_group:a", "spend:model_access_group:b"]
    );
    let sources: Vec<Option<&str>> = list
        .iter()
        .map(|counter| counter.source_cache_key.as_deref())
        .collect();
    assert_eq!(
        sources,
        vec![Some("model_access_group:a"), Some("model_access_group:b")]
    );
}

#[rstest]
#[case::missing(None)]
#[case::no_budget(Some("null"))]
#[case::zero(Some(r#"{"max_budget":0}"#))]
#[case::negative(Some(r#"{"max_budget":-2}"#))]
#[tokio::test]
async fn group_skip_zero_or_missing_budget(#[case] budget: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","matched_model_access_groups":["a"]}"#);
    if let Some(spec) = budget {
        fixture.rows = FakeRows::new().with_groups(vec![group_row("a", "0", spec)]);
    }
    assert!(fixture.counters().await.is_empty());
}
