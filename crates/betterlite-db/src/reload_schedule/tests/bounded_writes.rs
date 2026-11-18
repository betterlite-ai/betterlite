#![cfg(test)]

use std::time::Duration;

use deadpool_postgres::GenericClient as _;

use super::support::{Database, RecordingEvictor, hold_every_connection, instant, seed_row};
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{
    ReloadScheduleError, ScheduleDeadlines, clear_reload_interval, read_reload_schedule,
    record_manual_reload, write_reload_interval,
};

#[tokio::test]
async fn manual_reload_gives_up_when_the_pool_has_no_connection_left() {
    let database = Database::provision().await;
    let _held = hold_every_connection(&database).await;
    let evictor = RecordingEvictor::default();
    let deadlines = ScheduleDeadlines {
        acquire: Duration::from_millis(50),
        statement: Duration::from_secs(5),
    };
    let error = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        deadlines,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Timeout {
            operation: "acquire",
            ..
        }
    ));
    assert!(evictor.names().is_empty());
    database.close().await;
}

#[tokio::test]
async fn manual_reload_gives_up_when_the_config_row_stays_locked() {
    let database = Database::provision().await;
    seed_row(&database, None, None, 0).await;
    let blocker = database.store.session().await.unwrap();
    blocker.execute("BEGIN", &[]).await.unwrap();
    blocker
        .execute(
            r#"SELECT 1 FROM "LiteLLM_Config" WHERE "param_name" = $1 FOR UPDATE"#,
            &[&RELOAD_CONFIG_PARAM_NAME],
        )
        .await
        .unwrap();
    let evictor = RecordingEvictor::default();
    let deadlines = ScheduleDeadlines {
        acquire: Duration::from_secs(10),
        statement: Duration::from_millis(100),
    };
    let error = record_manual_reload(
        Some(database.client()),
        &evictor,
        instant().and_utc(),
        deadlines,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Timeout {
            operation: "record_manual_reload",
            ..
        }
    ));
    assert!(evictor.names().is_empty());
    drop(blocker);
    database.close().await;
}

#[tokio::test]
async fn repository_read_gives_up_when_the_pool_has_no_connection_left() {
    let database = Database::provision().await;
    let _held = hold_every_connection(&database).await;
    let deadlines = ScheduleDeadlines {
        acquire: Duration::from_millis(50),
        statement: Duration::from_millis(50),
    };
    let error = read_reload_schedule(Some(database.client()), deadlines)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReloadScheduleError::Timeout {
            operation: "read_reload_schedule",
            ..
        }
    ));
    database.close().await;
}

#[tokio::test]
async fn repository_writes_give_up_when_the_pool_has_no_connection_left() {
    let database = Database::provision().await;
    let _held = hold_every_connection(&database).await;
    let evictor = RecordingEvictor::default();
    let deadlines = ScheduleDeadlines {
        acquire: Duration::from_millis(50),
        statement: Duration::from_millis(50),
    };
    let write = write_reload_interval(Some(database.client()), &evictor, 8, deadlines)
        .await
        .unwrap_err();
    assert!(matches!(
        write,
        ReloadScheduleError::Timeout {
            operation: "write_reload_interval",
            ..
        }
    ));
    let clear = clear_reload_interval(Some(database.client()), &evictor, deadlines)
        .await
        .unwrap_err();
    assert!(matches!(
        clear,
        ReloadScheduleError::Timeout {
            operation: "clear_reload_interval",
            ..
        }
    ));
    assert!(evictor.names().is_empty());
    database.close().await;
}
