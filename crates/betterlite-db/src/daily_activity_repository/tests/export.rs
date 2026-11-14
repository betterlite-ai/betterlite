#![cfg(test)]

use super::support::{
    FakeDatabase, FakeFailure, FakeReads, RawReply, export_row, repository, scope, without,
};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{ExportRow, ExportType};
use crate::daily_activity_repository::export::next_cursor;
use rstest::rstest;
use rust_decimal::Decimal;

fn cursor_row(
    date: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    user_id: Option<&str>,
) -> ExportRow {
    ExportRow {
        date: date.to_owned(),
        entity_id: "entity".to_owned(),
        entity_alias: None,
        api_key: api_key.map(str::to_owned),
        key_alias: None,
        user_id: user_id.map(str::to_owned),
        user_email: None,
        model: model.map(str::to_owned),
        spend: Decimal::ZERO,
        flat_cost: Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        api_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    }
}

#[test]
fn cursor_uses_last_row() {
    let batch = vec![
        cursor_row("2026-08-01", Some("first"), Some("m1"), Some("u1")),
        cursor_row("2026-08-02", Some("last"), Some("m2"), Some("u2")),
    ];
    let cursor = next_cursor(&batch, ExportType::DailyWithKeys).unwrap();
    assert_eq!(cursor.date, "2026-08-02");
    assert_eq!(cursor.entity_id, "entity");
    assert_eq!(cursor.group_key, "last");
}

#[test]
fn cursor_each_export_group() {
    let batch = vec![cursor_row(
        "2026-08-02",
        Some("key"),
        Some("model"),
        Some("user"),
    )];
    assert_eq!(
        next_cursor(&batch, ExportType::DailyWithKeys)
            .unwrap()
            .group_key,
        "key"
    );
    assert_eq!(
        next_cursor(&batch, ExportType::DailyWithModels)
            .unwrap()
            .group_key,
        "model"
    );
    assert_eq!(
        next_cursor(&batch, ExportType::DailyWithUsers)
            .unwrap()
            .group_key,
        "user"
    );
    assert_eq!(
        next_cursor(&batch, ExportType::Daily).unwrap().group_key,
        ""
    );
}

#[rstest]
#[case::daily(ExportType::Daily)]
#[case::keys(ExportType::DailyWithKeys)]
#[case::models(ExportType::DailyWithModels)]
#[case::users(ExportType::DailyWithUsers)]
fn cursor_null_group_becomes_empty(#[case] export_type: ExportType) {
    let batch = vec![cursor_row("2026-08-02", None, None, None)];
    assert_eq!(next_cursor(&batch, export_type).unwrap().group_key, "");
}

#[tokio::test]
async fn export_short_batch_stops() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![export_row(1), export_row(2)]);
    let repository = repository(database, FakeReads::default());
    let mut rows = repository.export_rows(&scope(DailyActivityTable::User), ExportType::Daily);
    let mut seen = 0;
    while rows.next_row().await.unwrap().is_some() {
        seen += 1;
    }
    assert_eq!(seen, 2);
    assert_eq!(repository.database.query_count(), 1);
}

#[tokio::test]
async fn export_exact_thousand_fetches_empty_page() {
    let database = FakeDatabase::default();
    let batch: Vec<_> = (0..1000_usize).map(export_row).collect();
    database.set_raw_rows(batch);
    database.set_raw_rows(Vec::new());
    let repository = repository(database, FakeReads::default());
    let mut rows = repository.export_rows(&scope(DailyActivityTable::User), ExportType::Daily);
    let mut seen = 0;
    while rows.next_row().await.unwrap().is_some() {
        seen += 1;
    }
    assert_eq!(seen, 1000);
    assert_eq!(repository.database.query_count(), 2);
}

#[tokio::test]
async fn export_batch_validated_before_yield() {
    let database = FakeDatabase::default();
    database.set_raw_rows(vec![export_row(1), without(export_row(2), "flat_cost")]);
    let repository = repository(database, FakeReads::default());
    let mut rows = repository.export_rows(&scope(DailyActivityTable::User), ExportType::Daily);
    assert!(rows.next_row().await.is_err());
}

#[tokio::test]
async fn export_later_error_keeps_prior_yields() {
    let database = FakeDatabase::default();
    let batch: Vec<_> = (0..1000_usize).map(export_row).collect();
    database.set_raw_rows(batch);
    database.push_raw(RawReply::Failure(FakeFailure::Unavailable));
    let repository = repository(database, FakeReads::default());
    let mut rows = repository.export_rows(&scope(DailyActivityTable::User), ExportType::Daily);
    let mut seen = 0;
    while rows.next_row().await.unwrap().is_some() {
        seen += 1;
    }
    assert_eq!(seen, 1000);
    assert!(rows.next_row().await.is_err());
}

#[tokio::test]
async fn dropped_export_stops_queries() {
    let database = FakeDatabase::default();
    let batch: Vec<_> = (0..1000_usize).map(export_row).collect();
    database.set_raw_rows(batch);
    let repository = repository(database, FakeReads::default());
    let mut rows = repository.export_rows(&scope(DailyActivityTable::User), ExportType::Daily);
    assert!(rows.next_row().await.unwrap().is_some());
    drop(rows);
    assert_eq!(repository.database.query_count(), 1);
}
