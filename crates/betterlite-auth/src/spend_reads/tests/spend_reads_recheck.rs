use rust_decimal::Decimal;

use super::support::{
    Fixture, KEY, RowAnswer, aware, decimal, key_window, marker_key, request, window_row, written,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_reads::request::SpendReadRequest;
use crate::spend_reseed::values::ReseedCacheValue;

const WINDOW_KEY: &str = "spend:key:token:window:1h";

#[tokio::test]
async fn a_stale_low_counter_is_repaired_to_the_database_floor() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("10")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("10"));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("10")))
    );
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("10"))]
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("10"))]);
}

#[tokio::test]
async fn an_equal_fallback_is_not_a_staleness_signal() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("6")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("7")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("6"), Some(decimal("50"))))
        .await
        .expect("the equal fallback is not a staleness signal");

    assert_eq!(spend, decimal("6"));
    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(written(&fixture.cache.memory_writes(), &marker_key(KEY)).is_none());
}

#[tokio::test]
async fn a_healthy_counter_is_returned_without_a_database_read() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, Some(decimal("10"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(fixture.rows.entity_reads().is_empty());
    assert!(fixture.cache.memory_writes().is_empty());
}

#[tokio::test]
async fn a_window_counter_always_rechecks_against_its_window_floor() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(WINDOW_KEY, ReseedCacheValue::Number(decimal("3")));
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));
    let read_request = SpendReadRequest {
        window: Some(window),
        ..request(WINDOW_KEY, Decimal::ZERO, Some(decimal("100")))
    };

    let spend = fixture
        .reads(false)
        .get_current_spend(&read_request)
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("8"));
    assert_eq!(fixture.rows.window_reads(), ["Key:token:1h".to_owned()]);
    assert!(fixture.rows.entity_reads().is_empty());
}

#[tokio::test]
async fn a_floor_at_or_below_the_counter_keeps_the_counter() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("4")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("4")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("4"));
    assert!(fixture.cache.set_max_calls().is_empty());
    let marker =
        written(&fixture.cache.memory_writes(), &marker_key(KEY)).expect("the floor is cached");
    assert_eq!(marker.0, decimal("4"));
    assert_eq!(marker.1.map(f64::to_bits), Some(5.0_f64.to_bits()));
}

#[tokio::test]
async fn no_floor_returns_the_callers_higher_fallback() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("4")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(written(&fixture.cache.memory_writes(), &marker_key(KEY)).is_none());
}

#[tokio::test]
async fn spend_at_or_above_the_budget_returns_without_a_recheck() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("10")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("50")));

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, decimal("99"), Some(decimal("5"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("10"));
    assert!(fixture.rows.entity_reads().is_empty());
}

#[tokio::test]
async fn the_entity_floor_wins_over_the_window_floor() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));
    let read_request = SpendReadRequest {
        window: Some(window),
        ..request(KEY, decimal("5"), Some(decimal("100")))
    };

    let spend = fixture
        .reads(false)
        .get_current_spend(&read_request)
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("6"));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert!(fixture.rows.window_reads().is_empty());
}
