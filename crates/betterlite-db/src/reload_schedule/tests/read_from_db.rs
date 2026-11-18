#![cfg(test)]

use std::collections::BTreeMap;

use super::support::{Database, instant, seed_row, serialized_interval};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::reload_schedule::ReloadSchedule;
use crate::reload_schedule::parse_schedule::ParseScheduleError;
use crate::reload_schedule::store::{ReloadScheduleError, ScheduleDeadlines, read_reload_schedule};

#[tokio::test]
async fn read_from_db_returns_none_without_a_row() {
    let database = Database::provision().await;
    let schedule = read_reload_schedule(Some(database.client()), ScheduleDeadlines::default())
        .await
        .unwrap();
    assert_eq!(schedule, None);
    database.close().await;
}

#[tokio::test]
async fn read_from_db_parses_a_serialized_row() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(8)), Some(instant()), 3).await;
    let schedule = read_reload_schedule(Some(database.client()), ScheduleDeadlines::default())
        .await
        .unwrap();
    assert_eq!(
        schedule,
        Some(ReloadSchedule {
            interval_hours: Some(8),
            reload_revision: 3,
            last_run_at: Some(instant().and_utc()),
        })
    );
    database.close().await;
}

#[tokio::test]
async fn read_from_db_parses_a_decoded_object_row() {
    let database = Database::provision().await;
    let mut fields = BTreeMap::new();
    fields.insert(
        "interval_hours".to_owned(),
        JsonData::Number(serde_json::Number::from(6_i32)),
    );
    seed_row(&database, Some(JsonData::Object(fields)), None, 0).await;
    let schedule = read_reload_schedule(Some(database.client()), ScheduleDeadlines::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(schedule.interval_hours, Some(6));
    assert_eq!(schedule.last_run_at, None);
    database.close().await;
}

#[tokio::test]
async fn read_from_db_leaves_a_null_param_value_without_a_schedule() {
    let database = Database::provision().await;
    seed_row(&database, None, Some(instant()), 5).await;
    let schedule = read_reload_schedule(Some(database.client()), ScheduleDeadlines::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(schedule.interval_hours, None);
    assert_eq!(schedule.reload_revision, 5);
    assert_eq!(schedule.last_run_at, Some(instant().and_utc()));
    database.close().await;
}

#[tokio::test]
async fn read_from_db_reports_a_revision_past_u32() {
    let database = Database::provision().await;
    seed_row(
        &database,
        Some(serialized_interval(1)),
        None,
        i64::from(u32::MAX) + 1,
    )
    .await;
    let error = read_reload_schedule(Some(database.client()), ScheduleDeadlines::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Parse(ParseScheduleError::Revision { value })
            if value == i64::from(u32::MAX) + 1
    ));
    database.close().await;
}

#[tokio::test]
async fn read_from_db_reports_a_missing_database() {
    let error = read_reload_schedule(None, ScheduleDeadlines::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Database(DbError::NoDbConnected)
    ));
}
