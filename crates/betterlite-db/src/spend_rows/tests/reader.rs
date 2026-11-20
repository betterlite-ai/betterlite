use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::spend_rows::tests::support::{
    FakeRows, FixedDurations, HangingRows, decimal, reader_with, services,
};

#[rstest]
#[case::key("spend:key:k1", "key:k1")]
#[case::team_member(
    "spend:team_member:user:with:colon:team-1",
    "team_member:user:with:colon:team-1"
)]
#[case::team("spend:team:t1", "team:t1")]
#[case::user("spend:user:u1", "user:u1")]
#[case::organization("spend:org:o1", "org:o1")]
#[case::project("spend:project:p1", "project:p1")]
#[tokio::test]
async fn reads_the_primary_row(#[case] counter_key: &str, #[case] label: &str) {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        label.to_owned(),
        Some(decimal("2.5")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db(counter_key).await, Some(decimal("2.5")));
    assert_eq!(rows.calls(), vec![label.to_owned()]);
}

#[tokio::test]
async fn a_missing_row_returns_none() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::new()));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db("spend:key:k1").await, None);
    assert_eq!(rows.calls(), vec!["key:k1".to_owned()]);
}

#[tokio::test]
async fn a_null_spend_reads_as_zero() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "key:k1".to_owned(),
        None,
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db("spend:key:k1").await, Some(Decimal::ZERO));
}

#[tokio::test]
async fn a_missing_database_returns_none() {
    let reader = reader_with::<FakeRows>(None, Arc::new(FixedDurations), services(4), 1.0);
    assert_eq!(reader.from_db("spend:key:k1").await, None);
    assert_eq!(reader.end_user_from_db("spend:end_user:e1").await, None);
}

#[tokio::test]
async fn a_failed_row_read_returns_none() {
    let reader = reader_with(
        Some(Arc::new(FakeRows::failing())),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db("spend:key:k1").await, None);
    assert_eq!(reader.end_user_from_db("spend:end_user:e1").await, None);
}

#[tokio::test(start_paused = true)]
async fn a_lookup_past_its_deadline_returns_none() {
    let reader = reader_with(
        Some(Arc::new(HangingRows)),
        Arc::new(FixedDurations),
        services(4),
        0.05,
    );
    assert_eq!(reader.from_db("spend:key:k1").await, None);
}

#[tokio::test(start_paused = true)]
async fn a_primary_read_waits_for_the_lookup_gate() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "key:k1".to_owned(),
        Some(decimal("2.5")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(0),
        0.05,
    );
    assert_eq!(reader.from_db("spend:key:k1").await, None);
    assert!(rows.calls().is_empty());
}

#[rstest]
#[case::key("spend:key:k1:window:1h")]
#[case::team("spend:team:t1:window:30d")]
#[tokio::test]
async fn window_counters_are_not_primary_reads(#[case] counter_key: &str) {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([
        ("key:k1".to_owned(), Some(decimal("9"))),
        ("team:t1".to_owned(), Some(decimal("9"))),
    ])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db(counter_key).await, None);
    assert!(rows.calls().is_empty());
}

#[tokio::test]
async fn a_malformed_window_suffix_stays_a_primary_read() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "key:k1:window:soon".to_owned(),
        Some(decimal("2.5")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(
        reader.from_db("spend:key:k1:window:soon").await,
        Some(decimal("2.5"))
    );
    assert_eq!(rows.calls(), vec!["key:k1:window:soon".to_owned()]);
}

#[tokio::test]
async fn an_unknown_shape_returns_none() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::new()));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db("spend:model:m1").await, None);
    assert!(rows.calls().is_empty());
}

#[tokio::test]
async fn from_db_does_not_reseed_end_users() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "end_user:e1".to_owned(),
        Some(decimal("7.25")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.from_db("spend:end_user:e1").await, None);
    assert!(rows.calls().is_empty());
}

#[tokio::test]
async fn the_floor_reader_accepts_only_end_user_counters() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "end_user:e1".to_owned(),
        Some(decimal("7.25")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(
        reader.end_user_from_db("spend:end_user:e1").await,
        Some(decimal("7.25"))
    );
    assert_eq!(reader.end_user_from_db("spend:key:k1").await, None);
    assert_eq!(
        reader.end_user_from_db("spend:end_user:e1:window:1h").await,
        None
    );
    assert_eq!(
        rows.calls(),
        vec!["end_user:e1".to_owned(), "end_user:e1:window:1h".to_owned()]
    );
}

#[tokio::test]
async fn the_floor_reader_opens_no_lookup_gate() {
    let rows = Arc::new(FakeRows::with_rows(BTreeMap::from([(
        "end_user:e1".to_owned(),
        Some(decimal("7.25")),
    )])));
    let reader = reader_with(
        Some(Arc::clone(&rows)),
        Arc::new(FixedDurations),
        services(0),
        0.05,
    );
    assert_eq!(
        reader.end_user_from_db("spend:end_user:e1").await,
        Some(decimal("7.25"))
    );
}

#[tokio::test]
async fn the_floor_reader_returns_none_when_the_read_fails() {
    let reader = reader_with(
        Some(Arc::new(FakeRows::failing())),
        Arc::new(FixedDurations),
        services(4),
        1.0,
    );
    assert_eq!(reader.end_user_from_db("spend:end_user:e1").await, None);
}
