use rust_decimal::Decimal;

use super::support::{Captured, Fixture, KEY, RowAnswer, decimal};
use crate::spend_reseed::values::ReseedCacheValue;

const END_USER_KEY: &str = "spend:end_user:alice";
const WINDOW_KEY: &str = "spend:key:token:window:1h";

#[tokio::test]
async fn a_found_row_is_written_to_memory_and_redis_and_answers_true() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("9"))]
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("9"))]);
}

#[tokio::test]
async fn a_row_whose_spend_is_null_reseeds_zero() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Null);

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(Decimal::ZERO))
    );
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), Decimal::ZERO)]
    );
}

#[tokio::test]
async fn a_missing_row_answers_false_and_leaves_the_counter_alone() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("3")));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(!reseeded);
    assert!(fixture.cache.memory_writes().is_empty());
    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(fixture.batch.forgotten().is_empty());
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("3")))
    );
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("3")))
    );
}

#[tokio::test]
async fn a_failed_row_read_answers_false_and_writes_nothing() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture.rows.fail_entity();

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(!reseeded);
    assert!(fixture.cache.memory_writes().is_empty());
    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(logs.has("the counter row read failed"));
}

#[tokio::test]
async fn an_end_user_counter_has_no_primary_row_to_reseed_from() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("3")));

    let reseeded = fixture
        .reads(false)
        .reseed_spend_counter_from_db(END_USER_KEY)
        .await;

    assert!(!reseeded);
    assert!(fixture.rows.entity_reads().is_empty());
    assert!(fixture.cache.set_max_calls().is_empty());
}

#[tokio::test]
async fn a_window_counter_has_no_primary_row_to_reseed_from() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("3")));

    let reseeded = fixture
        .reads(false)
        .reseed_spend_counter_from_db(WINDOW_KEY)
        .await;

    assert!(!reseeded);
    assert!(fixture.rows.entity_reads().is_empty());
    assert!(fixture.rows.window_reads().is_empty());
    assert!(fixture.cache.set_max_calls().is_empty());
}

#[tokio::test]
async fn a_failed_memory_write_does_not_stop_the_redis_repair() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));
    fixture.cache.fail_memory_writes();

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("9"))]
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("9"))]);
}

#[tokio::test]
async fn a_failed_redis_raise_still_answers_true() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));
    fixture.cache.fail_set_max();

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert!(logs.has("Unable to repair stale spend counter"));
}

#[tokio::test]
async fn without_redis_only_the_memory_copy_is_reseeded() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(fixture.batch.forgotten().is_empty());
}

#[tokio::test]
async fn a_higher_redis_value_is_not_lowered_by_the_reseed() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("12")));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("12")))
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("12"))]);
}

#[tokio::test]
async fn the_reseed_forgets_the_batched_counter_before_the_raise() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    let events = fixture.events();
    let forget = events
        .iter()
        .position(|event| event == &format!("batch_forget:{KEY}"))
        .expect("the batch forget is recorded");
    let raise = events
        .iter()
        .position(|event| event == &format!("set_max:{KEY}:9"))
        .expect("the redis raise is recorded");
    assert!(forget < raise);
}

#[tokio::test]
async fn a_fresh_zero_row_raises_the_counter_to_zero() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(Decimal::ZERO));

    let reseeded = fixture.reads(false).reseed_spend_counter_from_db(KEY).await;

    assert!(reseeded);
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), Decimal::ZERO)]
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), Decimal::ZERO)]);
}
