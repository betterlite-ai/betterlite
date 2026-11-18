#![cfg(test)]

use super::support::{
    Database, RecordingEvictor, instant, seed_row, serialized_interval, stored_row,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{ReloadScheduleError, ScheduleDeadlines, record_manual_reload};

#[tokio::test]
async fn manual_reload_creates_the_row_at_revision_one() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    let revision = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    assert_eq!(revision, 1);
    let row = stored_row(&database).await.unwrap();
    assert_eq!(row.last_run_at, Some(instant()));
    assert_eq!(row.reload_revision, 1);
    assert_eq!(row.param_value, JsonData::Null);
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn manual_reload_increments_the_published_revision() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(6)), None, 3).await;
    let evictor = RecordingEvictor::default();
    let revision = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    assert_eq!(revision, 4);
    let row = stored_row(&database).await.unwrap();
    assert_eq!(row.reload_revision, 4);
    assert_eq!(row.last_run_at, Some(instant()));
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours": 6}"#.to_owned())
    );
    database.close().await;
}

#[tokio::test]
async fn manual_reload_publishes_distinct_revisions_for_concurrent_requests() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    let ran_at = instant().and_utc();
    let first = record_manual_reload(
        Some(database.client()),
        &evictor,
        ran_at,
        ScheduleDeadlines::default(),
    );
    let second = record_manual_reload(
        Some(database.client()),
        &evictor,
        ran_at,
        ScheduleDeadlines::default(),
    );
    let (left, right) = tokio::join!(first, second);
    let mut revisions = vec![left.unwrap(), right.unwrap()];
    revisions.sort_unstable();
    assert_eq!(revisions, vec![1, 2]);
    assert_eq!(stored_row(&database).await.unwrap().reload_revision, 2);
    database.close().await;
}

#[tokio::test]
async fn manual_reload_reports_a_missing_database_and_leaves_the_cache_alone() {
    let evictor = RecordingEvictor::default();
    let error = record_manual_reload(
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

#[tokio::test]
async fn manual_reload_reports_a_revision_past_u32() {
    let database = Database::provision().await;
    seed_row(&database, None, None, 4_294_967_295).await;
    let evictor = RecordingEvictor::default();
    let error = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Revision {
            value: 4_294_967_296
        }
    ));
    assert_eq!(
        stored_row(&database).await.unwrap().reload_revision,
        4_294_967_296
    );
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}
