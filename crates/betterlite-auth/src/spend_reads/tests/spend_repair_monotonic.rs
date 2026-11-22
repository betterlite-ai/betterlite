use super::support::{Captured, Fixture, KEY, decimal};
use crate::spend_reseed::values::ReseedCacheValue;

#[tokio::test]
async fn memory_is_raised_to_the_database_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("2")));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert_eq!(
        fixture.cache.memory_writes(),
        [(KEY.to_owned(), decimal("5"), None)]
    );
    assert!(!logs.has("could not be repaired"));
}

#[tokio::test]
async fn memory_is_never_lowered() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("7")));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("7")))
    );
    assert!(fixture.cache.memory_writes().is_empty());
}

#[tokio::test]
async fn memory_equal_to_the_floor_is_not_rewritten() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("5")));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert!(fixture.cache.memory_writes().is_empty());
}

#[tokio::test]
async fn an_unreadable_memory_value_is_overwritten() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Text("abc".to_owned()));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
}

#[tokio::test]
async fn an_absent_memory_value_is_set() {
    let fixture = Fixture::new(false);

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.memory_writes(),
        [(KEY.to_owned(), decimal("5"), None)]
    );
}

#[tokio::test]
async fn the_redis_raise_keeps_a_concurrent_increment_and_records_it() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("9")));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("5"))]
    );
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("9"))]);
}

#[tokio::test]
async fn the_redis_raise_records_the_resulting_value() {
    let fixture = Fixture::new(true);

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("5"))]);
}

#[tokio::test]
async fn the_batch_forgets_the_counter_before_the_redis_raise() {
    let fixture = Fixture::new(true);

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    let events = fixture.events();
    let forget = events
        .iter()
        .position(|event| event == &format!("batch_forget:{KEY}"))
        .expect("the batch forget is recorded");
    let raise = events
        .iter()
        .position(|event| event == &format!("set_max:{KEY}:5"))
        .expect("the redis raise is recorded");
    assert!(forget < raise);
    assert_eq!(fixture.batch.forgotten(), [KEY.to_owned()]);
}

#[tokio::test]
async fn a_failed_redis_raise_keeps_the_memory_repair() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("2")));
    fixture.cache.fail_set_max();

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert!(fixture.batch.recorded().is_empty());
    assert!(logs.has("Unable to repair stale spend counter"));
}

#[tokio::test]
async fn a_failed_memory_write_still_repairs_redis() {
    let fixture = Fixture::new(true);
    fixture.cache.fail_memory_writes();

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("5"))]
    );
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
}

#[tokio::test]
async fn a_failed_memory_read_is_treated_as_stale() {
    let fixture = Fixture::new(true);
    fixture.cache.fail_memory_reads();

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("5"))]
    );
}

#[tokio::test]
async fn without_redis_only_memory_is_repaired() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("1")));

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert!(fixture.cache.set_max_calls().is_empty());
    assert!(fixture.batch.forgotten().is_empty());
    assert!(fixture.batch.recorded().is_empty());
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
}

#[tokio::test]
async fn a_raise_that_answers_nothing_records_nothing() {
    let fixture = Fixture::new(true);
    fixture.cache.set_max_answers_nothing();

    fixture
        .reads(false)
        .repair_stale_spend_counter(KEY, decimal("5"))
        .await;

    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("5"))]
    );
    assert!(fixture.batch.recorded().is_empty());
    assert_eq!(fixture.cache.stored_redis(KEY), None);
}
