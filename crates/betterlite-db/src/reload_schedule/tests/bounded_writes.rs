#![cfg(test)]

use std::time::Duration;

use deadpool_postgres::GenericClient as _;

use super::support::{Database, RecordingEvictor, instant, seed_row};
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::store::{ReloadScheduleError, ScheduleDeadlines, record_manual_reload};

#[tokio::test]
async fn manual_reload_gives_up_when_the_pool_has_no_connection_left() {
    let database = Database::provision().await;
    // the pool holds four connections, so holding all four leaves the write waiting for a fifth
    let _first = database.store.session().await.unwrap();
    let _second = database.store.session().await.unwrap();
    let _third = database.store.session().await.unwrap();
    let _fourth = database.store.session().await.unwrap();
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
