#![cfg(test)]

use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, close_store, finish, insert_tier, moment, naive, past, schema, serial_pool,
};
use crate::budget_tier_sweep::tier_reads::read_due_tiers;
use crate::database_faults::ErrorType;
use crate::postgres_store::session::PostgresStore;

#[tokio::test]
async fn due_read_keeps_tiers_with_a_duration_and_no_reset_or_a_past_one() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "never-reset", Some("1d"), None).await;
    insert_tier(&client, "past", Some("1d"), Some(past())).await;
    insert_tier(&client, "future", Some("1d"), Some(naive(30))).await;
    insert_tier(&client, "no-duration", None, Some(past())).await;
    insert_tier(&client, "expiring-now", Some("1d"), Some(naive(0))).await;
    let mut reconnect = CountingReconnect::new(false);
    let due = read_due_tiers(&store, moment(0), 500, Some(&mut reconnect))
        .await
        .unwrap();
    let mut ids: Vec<String> = due
        .iter()
        .filter_map(|tier| tier.budget.budget_id.clone())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["never-reset".to_owned(), "past".to_owned()]);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn due_read_stops_at_the_chunk_limit() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "a", Some("1d"), Some(past())).await;
    insert_tier(&client, "b", Some("1d"), Some(past())).await;
    insert_tier(&client, "c", Some("1d"), Some(past())).await;
    let mut reconnect = CountingReconnect::new(false);
    let due = read_due_tiers(&store, moment(0), 2, Some(&mut reconnect))
        .await
        .unwrap();
    assert_eq!(due.len(), 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn due_read_retries_a_pool_timeout_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "due", Some("1d"), Some(past())).await;
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let mut reconnect = CountingReconnect::releasing(held);
    let due = read_due_tiers(&serial, moment(0), 500, Some(&mut reconnect))
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(
        reconnect.reasons(),
        vec![String::from("reset_budget_read_budgets_failure")]
    );
    close_store(serial);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn due_read_maps_a_pool_timeout_to_a_connect_fault() {
    let (fixture, store, client) = schema().await;
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let error = read_due_tiers(&serial, moment(0), 500, None::<&mut CountingReconnect>)
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Connect);
    drop(held);
    close_store(serial);
    finish(fixture, store, client).await;
}
