#![cfg(test)]

use super::support::{
    Database, RecordingEvictor, instant, seed_row, serialized_interval, stored_row,
};
use crate::budget_permission_rows::JsonData;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{ScheduleDeadlines, clear_reload_interval};

#[tokio::test]
async fn clear_interval_nulls_the_interval_inside_the_row() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(6)), Some(instant()), 4).await;
    let evictor = RecordingEvictor::default();
    clear_reload_interval(
        Some(database.client()),
        &evictor,
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    let row = stored_row(&database).await.unwrap();
    assert_eq!(
        row.param_value,
        JsonData::String(r#"{"interval_hours":null}"#.to_owned())
    );
    assert_eq!(row.last_run_at, Some(instant()));
    assert_eq!(row.reload_revision, 4);
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn clear_interval_does_not_create_a_missing_row() {
    let database = Database::provision().await;
    let evictor = RecordingEvictor::default();
    clear_reload_interval(
        Some(database.client()),
        &evictor,
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap();
    assert!(stored_row(&database).await.is_none());
    assert_eq!(evictor.names(), vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]);
    database.close().await;
}
