#![cfg(test)]

use super::support::{
    Database, FailingEvictor, instant, seed_row, serialized_interval, stored_row,
};
use crate::budget_permission_rows::JsonData;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::evictor::EvictError;
use crate::reload_schedule::store::{
    ReloadScheduleError, ScheduleDeadlines, clear_reload_interval, record_manual_reload,
    record_reload_run, write_reload_interval,
};

fn eviction_failure(error: ReloadScheduleError) -> EvictError {
    let ReloadScheduleError::Evict(failure) = error else {
        panic!("expected an eviction failure");
    };
    failure
}

#[tokio::test]
async fn write_interval_reports_a_failed_eviction_after_storing_the_interval() {
    let database = Database::provision().await;
    let evictor = FailingEvictor;
    let error = write_reload_interval(Some(database.client()), &evictor, 8)
        .await
        .unwrap_err();
    let failure = eviction_failure(error);
    assert_eq!(failure.param_name(), RELOAD_CONFIG_PARAM_NAME);
    assert_eq!(
        stored_row(&database).await.unwrap().param_value,
        JsonData::String(r#"{"interval_hours":8}"#.to_owned())
    );
    database.close().await;
}

#[tokio::test]
async fn clear_interval_reports_a_failed_eviction_after_clearing_the_interval() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(6)), Some(instant()), 4).await;
    let evictor = FailingEvictor;
    let error = clear_reload_interval(Some(database.client()), &evictor)
        .await
        .unwrap_err();
    let failure = eviction_failure(error);
    assert_eq!(failure.param_name(), RELOAD_CONFIG_PARAM_NAME);
    assert_eq!(
        stored_row(&database).await.unwrap().param_value,
        JsonData::String(r#"{"interval_hours":null}"#.to_owned())
    );
    database.close().await;
}

#[tokio::test]
async fn record_run_reports_a_failed_eviction_after_stamping_the_run() {
    let database = Database::provision().await;
    seed_row(&database, Some(serialized_interval(6)), None, 4).await;
    let evictor = FailingEvictor;
    let error = record_reload_run(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap_err();
    let failure = eviction_failure(error);
    assert_eq!(failure.param_name(), RELOAD_CONFIG_PARAM_NAME);
    assert_eq!(
        stored_row(&database).await.unwrap().last_run_at,
        Some(instant())
    );
    database.close().await;
}

#[tokio::test]
async fn manual_reload_reports_a_failed_eviction_after_publishing_the_revision() {
    let database = Database::provision().await;
    let evictor = FailingEvictor;
    let error = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        ScheduleDeadlines::default(),
    )
    .await
    .unwrap_err();
    let failure = eviction_failure(error);
    assert_eq!(failure.param_name(), RELOAD_CONFIG_PARAM_NAME);
    assert_eq!(stored_row(&database).await.unwrap().reload_revision, 1);
    database.close().await;
}
