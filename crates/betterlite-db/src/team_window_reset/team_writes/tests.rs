#![cfg(test)]

use chrono::{DateTime, Utc};

use crate::database_faults::ErrorType;
use crate::identity_budget_reset::write_transactional::ResetWrite;
use crate::postgres_store::session::PostgresStore;
use crate::team_window_reset::team_writes::{
    TEAM_WRITE_REASON, team_counter_keys, write_team_resets,
};
use crate::team_window_reset::tests::support::{
    CountingReconnect, TEAM_COLUMN, TEAM_TABLE, assert_spend, close_store, decimal, finish,
    insert_team, moment, past, reset_at_of, schema, serial_pool,
};

fn write(identifier: &str, reset_at: Option<DateTime<Utc>>, decrement: &str) -> ResetWrite {
    ResetWrite {
        identifier: identifier.to_owned(),
        budget_reset_at: reset_at,
        spend_decrement: decimal(decrement),
    }
}

#[tokio::test]
async fn team_writes_apply_every_reset_in_one_chunk() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0_f64), Some("1h"), Some(past())).await;
    insert_team(&client, "b", 2.5, None, Some("1d"), Some(past())).await;
    let writes = vec![
        write("a", Some(moment(3600)), "10"),
        write("b", Some(moment(7200)), "0"),
    ];
    write_team_resets(&store, writes, None::<&mut CountingReconnect>)
        .await
        .unwrap();
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 0.0).await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "b", 2.5).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "a").await,
        Some(moment(3600).naive_utc())
    );
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "b").await,
        Some(moment(7200).naive_utc())
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_writes_report_a_team_that_no_longer_exists() {
    let (fixture, store, client) = schema().await;
    let mut reconnect = CountingReconnect::new(true);
    let error = write_team_resets(
        &store,
        vec![write("missing", Some(moment(3600)), "0")],
        Some(&mut reconnect),
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert_eq!(
        error.message,
        "no row matched the reset target in LiteLLM_TeamTable"
    );
    assert_eq!(reconnect.attempts(), 0_usize);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_writes_leave_rows_alone_when_nothing_is_due() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0_f64), Some("1h"), Some(past())).await;
    write_team_resets(&store, Vec::new(), None::<&mut CountingReconnect>)
        .await
        .unwrap();
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 10.0).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "a").await,
        Some(past())
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_writes_retry_a_pool_timeout_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0_f64), Some("1h"), Some(past())).await;
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let mut reconnect = CountingReconnect::releasing(held);
    write_team_resets(
        &serial,
        vec![write("a", Some(moment(3600)), "10")],
        Some(&mut reconnect),
    )
    .await
    .unwrap();
    assert_eq!(reconnect.attempts(), 1_usize);
    assert_eq!(reconnect.reasons(), vec![TEAM_WRITE_REASON.to_owned()]);
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 0.0).await;
    close_store(serial);
    finish(fixture, store, client).await;
}

#[test]
fn team_counter_keys_drop_each_teams_counter() {
    let writes = vec![write("a", Some(moment(3600)), "0"), write("b", None, "0")];
    assert_eq!(
        team_counter_keys(&writes),
        vec!["spend:team:a".to_owned(), "spend:team:b".to_owned()]
    );
    assert!(team_counter_keys(&[]).is_empty());
}
