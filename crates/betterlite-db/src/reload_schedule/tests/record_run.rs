#![cfg(test)]

use super::support::{
    Database, RecordingEvictor, instant, seed_row, serialized_interval, stored_row,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{ReloadScheduleError, ScheduleDeadlines, record_reload_run};

#[tokio::test]
async fn record_run_stamps_the_last_run() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(6)), None, 4).await;
    let evictor = RecordingEvictor::default();
    record_reload_run(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    let row = stored_row(&database).await.unwrap();
    assert_eq!(row.last_run_at, Some(instant()));
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours": 6}"#.to_owned())
    );
    assert_eq!(row.reload_revision, 4);
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn record_run_leaves_no_row_behind_when_the_schedule_is_gone() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    record_reload_run(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    assert!(stored_row(&database).await.is_none());
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn record_run_reports_a_missing_database_and_leaves_the_cache_alone() {
    let evictor = RecordingEvictor::default();
    let error = record_reload_run(
        None,
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Database(DbError::NoDbConnected)
    ));
    assert!(evictor.names().is_empty());
}
