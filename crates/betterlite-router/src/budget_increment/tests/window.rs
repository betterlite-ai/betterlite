use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{Fixture, budget, flush, queued};
use crate::budget_increment::error::IncrementError;

fn cost() -> Decimal {
    Decimal::new(25, 2)
}

#[tokio::test]
async fn a_missing_start_time_is_set_to_now_with_the_full_duration() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap();
    assert_eq!(
        fixture.redis().start_writes(),
        vec![("start:a".to_owned(), 1_000.0_f64, Duration::from_secs(60))]
    );
    assert_eq!(fixture.memory_expiry("start:a"), Some(1_060.0_f64));
    assert_eq!(fixture.memory_expiry("spend:a"), Some(1_060.0_f64));
    assert!(fixture.memory_value("spend:a").is_some());
    assert!(flush(&fixture).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(), 60)]]
    );
}

#[tokio::test]
async fn an_active_window_increments_with_the_remaining_ttl() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap();
    assert!(fixture.redis().start_writes().is_empty());
    assert!(fixture.redis().spend_writes().is_empty());
    assert_eq!(fixture.memory_expiry("spend:a"), Some(1_060.0_f64));
    assert!(flush(&fixture).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(), 30)]]
    );
}

#[tokio::test]
async fn an_expired_window_resets_spend_and_start_time() {
    let fixture = Fixture::new(1_100.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap();
    assert_eq!(
        fixture.redis().spend_writes(),
        vec![("spend:a".to_owned(), cost(), Duration::from_secs(60))]
    );
    assert_eq!(
        fixture.redis().start_writes(),
        vec![("start:a".to_owned(), 1_100.0_f64, Duration::from_secs(60))]
    );
    assert_eq!(fixture.memory_expiry("spend:a"), Some(1_160.0_f64));
    assert_eq!(fixture.memory_expiry("start:a"), Some(1_160.0_f64));
    assert!(flush(&fixture).await);
    assert!(fixture.redis().batches().is_empty());
}

#[rstest]
#[case::window_start(1_000.0_f64, Some(60))]
#[case::halfway(1_030.0_f64, Some(30))]
#[case::a_fraction_below_the_end(1_059.5_f64, Some(0))]
#[case::exactly_at_the_end(1_060.0_f64, Some(0))]
#[case::just_past_the_end(1_060.5_f64, None)]
#[tokio::test]
async fn the_window_boundary_picks_the_increment_or_the_reset(
    #[case] now: f64,
    #[case] ttl: Option<u64>,
) {
    let fixture = Fixture::new(now);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap();
    if let Some(seconds) = ttl {
        assert!(fixture.redis().spend_writes().is_empty());
        assert!(flush(&fixture).await);
        assert_eq!(
            fixture.redis().batches(),
            vec![vec![queued("spend:a", cost(), seconds)]]
        );
    } else {
        assert_eq!(fixture.redis().spend_writes().len(), 1);
        assert!(flush(&fixture).await);
        assert!(fixture.redis().batches().is_empty());
    }
}

#[tokio::test]
async fn a_start_ahead_of_the_clock_extends_the_remaining_ttl() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().seed_start("start:a", 1_100.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap();
    assert!(flush(&fixture).await);
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", cost(), 160)]]
    );
}

#[tokio::test]
async fn a_budget_without_a_duration_skips_the_increment() {
    let fixture = Fixture::new(1_000.0_f64);
    let mut config = budget(60);
    config.budget_duration = None;
    fixture
        .increment
        .increment_spend_for_key(&config, "spend:a", "start:a", cost())
        .await
        .unwrap();
    assert!(fixture.redis().start_writes().is_empty());
    assert!(fixture.redis().spend_writes().is_empty());
    assert!(fixture.memory_value("spend:a").is_none());
    assert!(flush(&fixture).await);
    assert!(fixture.redis().batches().is_empty());
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::infinite(f64::INFINITY)]
#[case::negative(-1.0_f64)]
#[tokio::test]
async fn an_invalid_clock_is_rejected(#[case] now: f64) {
    let fixture = Fixture::new(now);
    let error = fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::InvalidSeconds { .. }));
}

#[tokio::test]
async fn a_cached_start_that_is_not_a_number_is_rejected() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .memory
        .lock()
        .unwrap()
        .set(
            "start:a",
            CacheValue::Text("nope".to_owned()),
            Some(600.0_f64),
        )
        .unwrap();
    let error = fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::InvalidCacheValue { key } if key == "start:a"));
}

#[tokio::test]
async fn a_negative_cached_start_is_rejected() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().seed_start("start:a", -5.0_f64);
    let error = fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::InvalidSeconds { .. }));
}

#[tokio::test]
async fn a_cached_spend_that_is_not_a_decimal_is_rejected() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .memory
        .lock()
        .unwrap()
        .set(
            "spend:a",
            CacheValue::Text("nope".to_owned()),
            Some(600.0_f64),
        )
        .unwrap();
    let error = fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", cost())
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::InvalidCacheValue { key } if key == "spend:a"));
}

#[tokio::test]
async fn a_spend_overflow_is_reported() {
    let fixture = Fixture::new(1_030.0_f64);
    fixture.redis().seed_start("start:a", 1_000.0_f64);
    fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", Decimal::MAX)
        .await
        .unwrap();
    let error = fixture
        .increment
        .increment_spend_for_key(&budget(60), "spend:a", "start:a", Decimal::ONE)
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::SpendOverflow { key } if key == "spend:a"));
}
