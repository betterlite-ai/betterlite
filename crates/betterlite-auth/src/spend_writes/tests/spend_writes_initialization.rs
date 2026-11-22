use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::error::SpendWriteError;

use super::support::{Fault, KEY, OTHER_KEY, decimal, fixture, source_json};

#[tokio::test]
async fn a_warm_batch_counter_skips_the_database() {
    let fx = fixture(true);
    fx.batch.answer_with(CounterValue::Amount(decimal("3")));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the warm counter initializes");
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("3")))
    );
    assert_eq!(fx.rows.spend_reads(), 0);
    assert!(!fx.journal.records(&format!("redis_get:{KEY}")));
}

#[tokio::test]
async fn a_batched_miss_reads_the_database_floor() {
    let fx = fixture(false);
    fx.batch.answer_with(CounterValue::Missing);
    fx.rows.hold_spend(decimal("6"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the cold counter reseeds");
    assert_eq!(fx.rows.spend_reads(), 1);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("6")))
    );
}

#[tokio::test]
async fn a_redis_hit_warms_the_counter() {
    let fx = fixture(true);
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("5")));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the redis counter is already warm");
    assert_eq!(fx.rows.spend_reads(), 0);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
}

#[tokio::test]
async fn a_cold_counter_seeds_from_its_database_row() {
    let fx = fixture(false);
    fx.rows.hold_spend(decimal("12.5"));
    fx.sources.put(KEY, source_json("{\"spend\": 4}"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the database floor seeds the counter");
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("12.5")))
    );
    assert_eq!(fx.journal.count("increment_local:"), 0);
}

#[tokio::test]
async fn a_cold_counter_with_redis_seeds_redis_from_the_database_row() {
    let fx = fixture(true);
    fx.rows.hold_spend(decimal("6"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the database floor seeds the Redis counter");
    assert_eq!(fx.rows.spend_reads(), 1);
    assert_eq!(
        fx.layers.redis_value(),
        Some(ReseedCacheValue::Number(decimal("6")))
    );
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("6")))
    );
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("6"))]);
}

#[tokio::test]
async fn a_missing_database_row_seeds_the_source_cache_spend() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": 7}"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the source cache supplies the floor");
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("7")));
    assert!(fx.journal.records(&format!("increment_local:{KEY}")));
}

#[tokio::test]
async fn a_zero_source_spend_leaves_the_counter_alone() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": 0}"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("a zero floor needs no increment");
    assert_eq!(fx.layers.increment_total(KEY), None);
    assert_eq!(fx.journal.count("increment_local:"), 0);
}

#[tokio::test]
async fn the_first_source_cache_key_with_a_value_wins() {
    let fx = fixture(false);
    fx.sources.put(OTHER_KEY, source_json("{\"spend\": 3}"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY, OTHER_KEY])
        .await
        .expect("the second source key supplies the floor");
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("3")));
}

#[tokio::test]
async fn an_unreachable_database_falls_back_to_the_source_cache() {
    let fx = fixture(false);
    fx.rows.fail();
    fx.sources.put(KEY, source_json("{\"spend\": 4}"));
    fx.writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect("the source cache covers an unreachable database");
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("4")));
}

#[tokio::test]
async fn an_unreadable_source_spend_propagates() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": [1]}"));
    let error = fx
        .writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect_err("the unreadable source spend fails the seed");
    assert!(matches!(error, SpendWriteError::Value(_)));
}

#[tokio::test]
async fn a_seed_write_failure_propagates() {
    let fx = fixture(false);
    fx.rows.hold_spend(decimal("2.5"));
    fx.layers.faults.add(Fault::MemorySet);
    let error = fx
        .writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect_err("the seed write failure surfaces");
    assert!(matches!(error, SpendWriteError::Reseed(_)));
}

#[tokio::test]
async fn a_batch_read_failure_propagates() {
    let fx = fixture(false);
    fx.batch.fail();
    let error = fx
        .writes
        .ensure_counter_seeded(KEY, &[KEY])
        .await
        .expect_err("the batch failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Batch(SpendCounterBatchError::Cancelled)
    ));
}
