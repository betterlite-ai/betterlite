#![cfg(test)]

use super::support::{
    DailyCall, FakeDatabase, FakeReads, RawReply, capture_logs, count, entity_row, grouping_row,
    key_spend_row, repository, scope, text_row, with_column, without,
};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_repository::DailyActivityRepositoryError;
use crate::daily_activity_repository::database::DailyRowFilter;
use crate::daily_activity_repository::rows::DatabaseValue;
use rstest::rstest;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Barrier;

#[tokio::test]
async fn query_debug_first_line_only() {
    let (logs, subscriber) = capture_logs();
    let guard = tracing::subscriber::set_default(subscriber);
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    repository
        .aggregated(&scope(DailyActivityTable::User), false, 10)
        .await
        .unwrap();
    drop(guard);
    let contents = logs.contents();
    assert!(contents.contains("DailyActivityRepository query: SELECT date"));
    assert!(!contents.contains("SUM(prompt_tokens)"));
}

#[tokio::test]
async fn query_none_is_empty() {
    let database = FakeDatabase::default();
    database.push_raw(RawReply::None);
    let repository = repository(database, FakeReads::default());
    let page = repository
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 0);
    assert!(page.rows.is_empty());
}

#[tokio::test]
async fn aggregated_entity_query_optional() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let without_breakdown = repository
        .aggregated(&scope(DailyActivityTable::User), false, 10)
        .await
        .unwrap();
    assert!(without_breakdown.entity_rows.is_none());
    assert_eq!(repository.database.query_count(), 1);
    let with_breakdown = repository
        .aggregated(&scope(DailyActivityTable::User), true, 10)
        .await
        .unwrap();
    assert_eq!(with_breakdown.entity_rows, Some(Vec::new()));
    assert_eq!(repository.database.query_count(), 3);
}

#[tokio::test]
async fn aggregate_queries_concurrent() {
    let mut database = FakeDatabase::default();
    database.aggregate_barrier = Some(Arc::new(Barrier::new(2)));
    let repository = repository(database, FakeReads::default());
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        repository.aggregated(&scope(DailyActivityTable::User), true, 10),
    )
    .await;
    assert!(outcome.is_ok());
    assert_eq!(repository.database.query_count(), 2);
}

#[tokio::test]
async fn first_nonnull_distinct_count_including_zero() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![
        grouping_row(None),
        grouping_row(Some(0)),
        grouping_row(Some(7)),
    ]);
    let repository = repository(database, FakeReads::default());
    let first = repository
        .aggregated(&scope(DailyActivityTable::User), false, 10)
        .await
        .unwrap();
    assert_eq!(first.distinct_api_keys, 0);
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![grouping_row(None)]);
    let repository = repository(database, FakeReads::default());
    let none = repository
        .aggregated(&scope(DailyActivityTable::User), false, 10)
        .await
        .unwrap();
    assert_eq!(none.distinct_api_keys, 0);
}

#[tokio::test]
async fn invalid_grouping_before_invalid_entity() {
    let database = FakeDatabase::default();
    database.push_raw(RawReply::Rows(vec![without(
        grouping_row(None),
        "group_level",
    )]));
    database.push_raw(RawReply::Rows(vec![without(
        entity_row(Some("entity"), 0),
        "entity_id",
    )]));
    let repository = repository(database, FakeReads::default());
    let error = repository
        .aggregated(&scope(DailyActivityTable::User), true, 10)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("group_level"));
}

#[rstest]
#[case::zero(0)]
#[case::over_maximum(101)]
#[tokio::test]
async fn all_three_limits_one_hundred_and_reject(#[case] limit: i64) {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let scope = scope(DailyActivityTable::User);
    assert!(matches!(
        repository.search_keys(&scope, "key", limit).await,
        Err(DailyActivityRepositoryError::Limit { maximum: 100 })
    ));
    assert!(matches!(
        repository
            .model_top_keys(&scope, "group", false, limit)
            .await,
        Err(DailyActivityRepositoryError::Limit { maximum: 100 })
    ));
    assert!(matches!(
        repository.cache_leakage_keys(&scope, limit).await,
        Err(DailyActivityRepositoryError::Limit { maximum: 100 })
    ));
    assert_eq!(repository.database.query_count(), 0);
}

#[rstest]
#[case::minimum(1)]
#[case::maximum(100)]
#[tokio::test]
async fn all_three_limits_accept_one_and_one_hundred(#[case] limit: i64) {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let scope = scope(DailyActivityTable::User);
    repository.search_keys(&scope, "key", limit).await.unwrap();
    repository
        .model_top_keys(&scope, "group", false, limit)
        .await
        .unwrap();
    repository.cache_leakage_keys(&scope, limit).await.unwrap();
    assert_eq!(repository.database.query_count(), 3);
}

#[tokio::test]
async fn search_validates_discarded_metric_fields() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![with_column(
        key_spend_row("key"),
        "prompt_tokens",
        DatabaseValue::Text("not a number".to_owned()),
    )]);
    let repository = repository(database, FakeReads::default());
    let error = repository
        .search_keys(&scope(DailyActivityTable::User), "key", 5)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("prompt_tokens"));
}

#[tokio::test]
async fn key_page_first_count_only() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![
        with_column(key_spend_row("a"), "total_api_keys", count(2)),
        with_column(key_spend_row("b"), "total_api_keys", count(9)),
    ]);
    let repository = repository(database, FakeReads::default());
    let page = repository
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 2);
    assert_eq!(page.rows.len(), 2);
}

#[tokio::test]
async fn key_page_missing_null_count_zero() {
    let missing = FakeDatabase::default();
    missing.set_raw_rows(vec![key_spend_row("a")]);
    let page = repository(missing, FakeReads::default())
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 0);
    let null = FakeDatabase::default();
    null.set_raw_rows(vec![with_column(
        key_spend_row("a"),
        "total_api_keys",
        DatabaseValue::Null,
    )]);
    let page = repository(null, FakeReads::default())
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 0);
    let empty = FakeDatabase::default();
    let page = repository(empty, FakeReads::default())
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 0);
    assert!(page.rows.is_empty());
}

#[tokio::test]
async fn key_page_null_key_placeholder_removed() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![text_row(vec![
        ("total_api_keys", count(5)),
        ("api_key", DatabaseValue::Null),
    ])]);
    let repository = repository(database, FakeReads::default());
    let page = repository
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.total_api_keys, 5);
    assert!(page.rows.is_empty());
}

#[tokio::test]
async fn key_page_empty_key_retained() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![with_column(
        key_spend_row(""),
        "total_api_keys",
        count(1),
    )]);
    let repository = repository(database, FakeReads::default());
    let page = repository
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows.first().unwrap().api_key, "");
}

#[tokio::test]
async fn malformed_retained_row_fails() {
    let bad_count = FakeDatabase::default();
    bad_count.set_raw_rows(vec![with_column(
        key_spend_row("key"),
        "total_api_keys",
        DatabaseValue::Text("many".to_owned()),
    )]);
    let error = repository(bad_count, FakeReads::default())
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("total_api_keys"));
    let bad_row = FakeDatabase::default();
    bad_row.set_raw_rows(vec![without(
        with_column(key_spend_row("key"), "total_api_keys", count(1)),
        "completion_tokens",
    )]);
    let error = repository(bad_row, FakeReads::default())
        .key_page(&scope(DailyActivityTable::User), 0, 10)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("completion_tokens"));
}

#[tokio::test]
async fn daily_rows_sentinel_retained() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    repository
        .daily_rows(&scope(DailyActivityTable::User), 1, 10)
        .await
        .unwrap();
    let calls = repository.database.daily_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    let call: &DailyCall = calls.first().unwrap();
    assert_eq!(call.table, DailyActivityTable::User);
    assert_eq!(call.skip, 0);
    assert_eq!(call.take, 10);
}

#[tokio::test]
async fn pagination_arithmetic_checked() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    let error = repository
        .daily_rows(&scope(DailyActivityTable::User), i64::MAX, i64::MAX)
        .await
        .unwrap_err();
    assert!(matches!(error, DailyActivityRepositoryError::Pagination));
    assert_eq!(repository.database.daily_call_count(), 0);
}

#[tokio::test]
async fn count_and_rows_share_predicates() {
    let database = FakeDatabase::default();
    let repository = repository(database, FakeReads::default());
    repository
        .daily_rows(&scope(DailyActivityTable::Team), 2, 5)
        .await
        .unwrap();
    let calls = repository.database.daily_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls.first().unwrap().filters,
        calls.last().unwrap().filters
    );
    assert_eq!(calls.last().unwrap().skip, 5);
    let filter = calls.first().unwrap().filters.first().unwrap();
    assert!(matches!(filter, DailyRowFilter::DateRange { .. }));
}

#[tokio::test]
async fn daily_rows_count_barrier() {
    let mut database = FakeDatabase::default();
    database.daily_barrier = Some(Arc::new(Barrier::new(2)));
    let repository = repository(database, FakeReads::default());
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        repository.daily_rows(&scope(DailyActivityTable::User), 1, 10),
    )
    .await;
    assert!(outcome.is_ok());
    assert_eq!(repository.database.daily_call_count(), 2);
}
