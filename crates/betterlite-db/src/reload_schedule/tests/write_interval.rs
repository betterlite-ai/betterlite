#![cfg(test)]

use super::support::{
    Database, RecordingEvictor, instant, seed_row, serialized_interval, stored_row,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{
    ReloadScheduleError, ScheduleDeadlines, write_reload_interval,
};

#[tokio::test]
async fn write_interval_stores_the_interval_as_a_serialized_string() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    write_reload_interval(
        Some(database.client()),
        &evictor,
        8,
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    let row = stored_row(&database).await.unwrap();
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours":8}"#.to_owned())
    );
    assert_eq!(row.last_run_at, None);
    assert_eq!(row.reload_revision, 0);
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn write_interval_replaces_the_interval_and_keeps_the_tracking_columns() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(2)), Some(instant()), 4).await;
    let evictor = RecordingEvictor::default();
    write_reload_interval(
        Some(database.client()),
        &evictor,
        9,
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    let row = stored_row(&database).await.unwrap();
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours":9}"#.to_owned())
    );
    assert_eq!(row.last_run_at, Some(instant()));
    assert_eq!(row.reload_revision, 4);
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn write_interval_fails_without_a_database_and_leaves_the_cache_alone() {
    let evictor = RecordingEvictor::default();
    let error = write_reload_interval(None, &evictor, 8, ScheduleDeadlines::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Database(DbError::NoDbConnected)
    ));
    assert!(evictor.names().is_empty());
}

#[tokio::test]
async fn write_interval_stores_the_largest_interval() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    write_reload_interval(
        Some(database.client()),
        &evictor,
        u32::MAX,
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    let row = stored_row(&database).await.unwrap();
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours":4294967295}"#.to_owned())
    );
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}
