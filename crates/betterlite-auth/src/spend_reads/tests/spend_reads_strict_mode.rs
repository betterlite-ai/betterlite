use rust_decimal::Decimal;

use super::support::{Captured, Fixture, KEY, RowAnswer, decimal, request};
use crate::spend_reads::error::BUDGET_UNVERIFIABLE_STATUS;
use crate::spend_reads::request::SpendReadRequest;
use crate::spend_reseed::values::ReseedCacheValue;

#[tokio::test]
async fn strict_mode_rechecks_without_a_staleness_signal() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, Decimal::ZERO, Some(decimal("10"))))
        .await
        .expect("the read answers");

    assert_eq!(spend, decimal("9"));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert_eq!(
        fixture.cache.set_max_calls(),
        [(KEY.to_owned(), decimal("9"))]
    );
}

#[tokio::test]
async fn unverified_spend_is_rejected_with_503() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);

    let error = fixture
        .reads(true)
        .get_current_spend(&request(KEY, Decimal::ZERO, Some(decimal("10"))))
        .await
        .expect_err("the spend cannot be verified");

    assert_eq!(error.counter_key, KEY);
    assert_eq!(BUDGET_UNVERIFIABLE_STATUS, 503);
    assert!(
        error
            .to_string()
            .starts_with("Budget enforcement unavailable: current spend could not be verified")
    );
    assert!(logs.has("fail_closed_budget_enforcement"));
}

#[tokio::test]
async fn a_database_floor_verifies_a_counter_redis_could_not_answer() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture.cache.fail_redis();
    fixture.rows.set_entity(RowAnswer::Spent(decimal("2")));

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, Decimal::ZERO, Some(decimal("10"))))
        .await
        .expect("the database verifies the spend");

    assert_eq!(spend, decimal("2"));
    assert!(logs.has("Redis spend counter read failed"));
}

#[tokio::test]
async fn a_caller_fallback_marked_authoritative_is_admitted() {
    let fixture = Fixture::new(false);
    let read_request = SpendReadRequest {
        fallback_authoritative: true,
        ..request(KEY, decimal("3"), Some(decimal("10")))
    };

    let spend = fixture
        .reads(true)
        .get_current_spend(&read_request)
        .await
        .expect("the caller vouches for the fallback");

    assert_eq!(spend, decimal("3"));
}

#[tokio::test]
async fn an_authoritative_floor_at_the_estimate_verifies_it() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("4")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("4")));

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the floor verifies the per-pod count");

    assert_eq!(spend, decimal("4"));
}

#[tokio::test]
async fn a_strict_read_admits_the_callers_higher_fallback_when_no_floor_exists() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("4")));

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, decimal("5"), Some(decimal("100"))))
        .await
        .expect("the caller's fallback is the authoritative recorded spend");

    assert_eq!(spend, decimal("5"));
}

#[tokio::test]
async fn strict_mode_never_rejects_when_no_budget_is_enforced() {
    let fixture = Fixture::new(false);

    let spend = fixture
        .reads(true)
        .get_current_spend(&request(KEY, Decimal::ZERO, None))
        .await
        .expect("no budget means no verification");

    assert_eq!(spend, Decimal::ZERO);
}
