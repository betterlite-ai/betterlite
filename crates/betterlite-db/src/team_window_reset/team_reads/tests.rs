#![cfg(test)]

use crate::database_faults::ErrorType;
use crate::postgres_store::session::PostgresStore;
use crate::team_window_reset::team_reads::read_due_teams;
use crate::team_window_reset::tests::support::{
    CountingReconnect, close_store, finish, insert_team, moment, past, provision, schema,
    serial_pool,
};

#[tokio::test]
async fn team_reads_pick_up_due_durations_and_missing_reset_times() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "due", 1.0, Some(5.0_f64), Some("1h"), Some(past())).await;
    insert_team(
        &client,
        "fresh",
        1.0,
        Some(5.0_f64),
        Some("1h"),
        Some(moment(600).naive_utc()),
    )
    .await;
    insert_team(&client, "uninitialised", 2.0, None, Some("1d"), None).await;
    insert_team(&client, "static", 3.0, None, None, Some(past())).await;
    let due = read_due_teams(&store, moment(0), 10, None::<&mut CountingReconnect>)
        .await
        .unwrap();
    let mut identifiers: Vec<String> = due.iter().map(|row| row.identifier.clone()).collect();
    identifiers.sort();
    assert_eq!(
        identifiers,
        vec!["due".to_owned(), "uninitialised".to_owned()]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_reads_decode_spend_cap_duration_and_reset_time() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "due", 4.5, Some(9.5_f64), Some("2h"), Some(past())).await;
    let due = read_due_teams(&store, moment(0), 10, None::<&mut CountingReconnect>)
        .await
        .unwrap();
    assert_eq!(due.len(), 1_usize);
    let row = due.first().unwrap();
    assert_eq!(row.spend.to_string(), "4.5");
    assert_eq!(
        row.max_budget.map(|cap| cap.to_string()),
        Some("9.5".to_owned())
    );
    assert_eq!(row.budget_duration, "2h");
    assert_eq!(row.budget_reset_at, Some(past().and_utc()));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_reads_respect_the_batch_limit() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 1.0, None, Some("1h"), Some(past())).await;
    insert_team(&client, "b", 1.0, None, Some("1h"), Some(past())).await;
    insert_team(&client, "c", 1.0, None, Some("1h"), Some(past())).await;
    let due = read_due_teams(&store, moment(0), 2, None::<&mut CountingReconnect>)
        .await
        .unwrap();
    assert_eq!(due.len(), 2_usize);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_reads_surface_a_missing_table_as_an_other_fault() {
    let (fixture, store, client) = provision(vec!["SELECT 1".to_owned()]).await;
    let error = read_due_teams(&store, moment(0), 10, None::<&mut CountingReconnect>)
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn team_reads_retry_a_pool_timeout_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "due", 1.0, None, Some("1h"), Some(past())).await;
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let mut reconnect = CountingReconnect::releasing(held);
    let due = read_due_teams(&serial, moment(0), 10, Some(&mut reconnect))
        .await
        .unwrap();
    assert_eq!(due.len(), 1_usize);
    assert_eq!(reconnect.attempts(), 1_usize);
    assert_eq!(
        reconnect.reasons(),
        vec!["reset_budget_read_teams_failure".to_owned()]
    );
    close_store(serial);
    finish(fixture, store, client).await;
}
