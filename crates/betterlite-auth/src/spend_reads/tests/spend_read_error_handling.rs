use rust_decimal::Decimal;

use super::support::{
    Captured, Fixture, KEY, RowAnswer, aware, decimal, key_window, request, window_row,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_reads::error::BUDGET_UNVERIFIABLE_STATUS;
use crate::spend_reads::request::SpendReadRequest;
use crate::spend_reseed::values::ReseedCacheValue;

const WINDOW_KEY: &str = "spend:key:token:window:1h";

#[tokio::test]
async fn a_failed_batch_read_falls_through_to_redis() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture.batch.fail();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(logs.has("the spend counter batch read failed; reading Redis directly"));
}

#[tokio::test]
async fn a_failed_redis_read_falls_back_to_memory() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.cache.fail_redis();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("3"));
    assert!(logs.has("the Redis spend counter read failed; falling back to memory"));
}

#[tokio::test]
async fn an_unreadable_redis_value_falls_back_to_memory() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Text("abc".to_owned()));
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("3"));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn a_failed_memory_read_reads_as_absent() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.cache.fail_memory_reads();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("7"));
    assert!(logs.has("the in-memory spend counter read failed"));
}

#[tokio::test]
async fn an_unreadable_memory_value_reads_as_absent() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Text("abc".to_owned()));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("7"));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn a_failed_reseed_logs_and_uses_the_caller_fallback() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.batch.fail();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("7"));
    assert!(logs.has("the spend counter reseed failed; using the caller's fallback"));
}

#[tokio::test]
async fn a_failed_database_read_logs_and_uses_the_caller_fallback() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture.rows.fail_entity();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("7"));
    assert!(logs.has("the counter row read failed"));
}

#[tokio::test]
async fn a_failed_floor_read_keeps_the_callers_higher_fallback() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.rows.fail_entity();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("10"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(logs.has("the counter row read failed"));
}

#[tokio::test]
async fn a_failed_window_floor_read_keeps_the_fallback() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "\"abc\""));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));
    let read_request = SpendReadRequest {
        window: Some(window),
        ..request(WINDOW_KEY, decimal("5"), Some(decimal("100")))
    };

    let spend = fixture
        .reads(false)
        .get_current_spend(&read_request)
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(logs.has("the window spend floor read failed"));
}

#[tokio::test]
async fn a_failed_marker_write_does_not_stop_the_raised_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("10")));
    fixture.cache.fail_memory_writes();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("10"));
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("10"))]
    );
    assert!(logs.has("the spend floor marker could not be cached"));
    assert!(logs.has("the stale spend counter's in-memory copy could not be repaired"));
}

#[tokio::test]
async fn a_failed_redis_repair_keeps_the_database_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("10")));
    fixture.cache.fail_set_max();

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("10"));
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("10")))
    );
    assert!(logs.has("Unable to repair stale spend counter in Redis"));
}

#[tokio::test]
async fn a_failed_redis_read_in_strict_mode_rejects_the_request() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture.cache.fail_redis();

    let error = fixture
        .reads(true)
        .get_current_spend(&request(KEY, Decimal::ZERO, Some(decimal("10"))))
        .await
        .expect_err("nothing verified the spend");

    assert_eq!(error.counter_key, KEY);
    assert_eq!(BUDGET_UNVERIFIABLE_STATUS, 503);
    assert!(logs.has("the Redis spend counter read failed"));
}
