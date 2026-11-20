use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{FakeRows, Fixture, dec, tag_row};

#[tokio::test]
async fn tag_dedup_keeps_first_seen_order_and_queries_once() {
    let mut fixture = Fixture::new();
    fixture.tag_names = vec![
        "b".to_owned(),
        "a".to_owned(),
        "b".to_owned(),
        "a".to_owned(),
    ];
    fixture.rows = FakeRows::new().with_tags(vec![
        tag_row("a", "1", r#"{"max_budget":2}"#),
        tag_row("b", "2", r#"{"max_budget":3}"#),
    ]);
    let list = fixture.counters().await;
    let names: Vec<&str> = list
        .iter()
        .map(|counter| counter.entity_id.as_str())
        .collect();
    assert_eq!(names, vec!["b", "a"]);
    assert_eq!(
        fixture.rows.tag_queries(),
        vec![vec!["b".to_owned(), "a".to_owned()]]
    );
}

#[tokio::test]
async fn tag_batch_merges_the_cached_and_fetched_rows() {
    let mut fixture = Fixture::new();
    fixture.tag_names = vec!["a".to_owned(), "b".to_owned()];
    fixture
        .cache
        .put("tag:a", &tag_row("a", "1", r#"{"max_budget":2}"#));
    fixture.rows = FakeRows::new().with_tags(vec![tag_row("b", "3", r#"{"max_budget":4}"#)]);
    let list = fixture.counters().await;
    let caps: Vec<Decimal> = list.iter().map(|counter| counter.max_budget).collect();
    assert_eq!(caps, vec![dec("2"), dec("4")]);
    assert_eq!(fixture.rows.tag_queries(), vec![vec!["b".to_owned()]]);
    let keys = fixture.cache.read_keys();
    let reads: Vec<&str> = keys
        .iter()
        .filter(|key| key.starts_with("tag:"))
        .map(String::as_str)
        .collect();
    assert_eq!(reads, vec!["tag:a", "tag:b"]);
}

#[rstest]
#[case::missing(None)]
#[case::no_budget(Some("null"))]
#[case::zero(Some(r#"{"max_budget":0}"#))]
#[case::negative(Some(r#"{"max_budget":-2}"#))]
#[tokio::test]
async fn tag_skip_zero_or_missing_budget(#[case] budget: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.tag_names = vec!["a".to_owned()];
    if let Some(spec) = budget {
        fixture.rows = FakeRows::new().with_tags(vec![tag_row("a", "0", spec)]);
    }
    assert!(fixture.counters().await.is_empty());
}
