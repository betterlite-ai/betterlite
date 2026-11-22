use crate::budget_counter::CounterEntityType;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::values::ReseedCacheValue;

use super::support::{KEY, Logs, aware, decimal, fixture, window_request, window_row};

#[tokio::test]
async fn a_warm_window_counter_answers_before_the_database() {
    let fx = fixture(true);
    let entity = CounterEntityType::Key;
    let request = window_request(&entity, "k1");
    fx.batch.answer_with(CounterValue::Amount(decimal("2")));
    let warm = fx
        .writes
        .ensure_window_spend_counter_initialized(&request)
        .await
        .expect("the warm counter initializes");
    assert!(warm);
    assert_eq!(fx.rows.window_reads(), 0);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("2")))
    );
}

#[tokio::test]
async fn a_batched_miss_does_not_count_as_warm() {
    let fx = fixture(false);
    let entity = CounterEntityType::Key;
    let request = window_request(&entity, "k1");
    fx.batch.answer_with(CounterValue::Missing);
    fx.rows
        .hold_window(window_row(aware("2026-11-18T12:00:00+00:00"), "8"));
    let warm = fx
        .writes
        .ensure_window_spend_counter_initialized(&request)
        .await
        .expect("the batched miss reseeds the counter");
    assert!(warm);
    assert_eq!(fx.rows.window_reads(), 1);
}

#[tokio::test]
async fn a_window_row_warms_the_counter() {
    let fx = fixture(false);
    let entity = CounterEntityType::Key;
    let request = window_request(&entity, "k1");
    fx.rows
        .hold_window(window_row(aware("2026-11-18T12:00:00+00:00"), "8"));
    let warm = fx
        .writes
        .ensure_window_spend_counter_initialized(&request)
        .await
        .expect("the window row warms the counter");
    assert!(warm);
    assert_eq!(fx.rows.window_reads(), 1);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("8")))
    );
}

#[tokio::test]
async fn an_entity_without_window_rows_returns_false_and_warns() {
    let logs = Logs::capture();
    let fx = fixture(false);
    let entity = CounterEntityType::User;
    let request = window_request(&entity, "u1");
    let warm = fx
        .writes
        .ensure_window_spend_counter_initialized(&request)
        .await
        .expect("the missing window spend is an answer, not an error");
    assert!(!warm);
    assert_eq!(fx.rows.window_reads(), 0);
    assert!(logs.contains("Skipping cold spend counter seed"));
}
