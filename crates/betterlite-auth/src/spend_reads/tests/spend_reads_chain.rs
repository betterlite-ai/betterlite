use rust_decimal::Decimal;

use super::support::{Fixture, KEY, RowAnswer, decimal, request};
use crate::spend_reseed::values::ReseedCacheValue;

#[tokio::test]
async fn redis_answers_before_memory_and_the_database() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("5"));
    assert!(fixture.rows.entity_reads().is_empty());
    assert_eq!(
        fixture.events(),
        [format!("batch_read:{KEY}"), format!("redis_get:{KEY}")]
    );
}

#[tokio::test]
async fn in_memory_answers_when_redis_is_not_configured() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("3"));
    assert!(fixture.rows.entity_reads().is_empty());
    assert_eq!(
        fixture.events(),
        [format!("batch_read:{KEY}"), format!("memory_get:{KEY}")]
    );
}

#[tokio::test]
async fn a_clean_redis_miss_skips_memory_and_reseeds_from_the_database() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("9"));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert_eq!(
        fixture.cache.stored_redis(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert_eq!(
        fixture.cache.stored_memory(KEY),
        Some(ReseedCacheValue::Number(decimal("9")))
    );
    assert_eq!(fixture.batch.recorded(), [(KEY.to_owned(), decimal("9"))]);
    assert_eq!(fixture.cache.nx_writes(), [(KEY.to_owned(), decimal("9"))]);
}

#[tokio::test]
async fn a_cold_counter_uses_the_callers_fallback() {
    let fixture = Fixture::new(true);

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("7"));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert_eq!(fixture.cache.stored_redis(KEY), None);
}

#[tokio::test]
async fn a_fresh_zero_row_answers_zero_rather_than_the_fallback() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(Decimal::ZERO));

    let spend = fixture
        .reads(false)
        .get_current_spend(&request(KEY, decimal("7"), None))
        .await
        .expect("the read answers");

    assert_eq!(spend, Decimal::ZERO);
}

#[tokio::test]
async fn a_read_without_a_request_batch_still_reseeds_from_the_database() {
    let fixture = Fixture::new(true);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads_without_batch(false)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("9"));
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.starts_with("batch_read:"))
    );
}
