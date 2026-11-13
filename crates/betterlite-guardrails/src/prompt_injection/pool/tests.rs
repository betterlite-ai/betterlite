#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use super::HeuristicsPool;
use crate::prompt_injection::error::PromptInjectionError;

fn threshold() -> Decimal {
    "0.7".parse().expect("the test threshold is a decimal")
}

fn check_timeout() -> Duration {
    Duration::from_secs(30)
}

fn keywords(values: &[&str]) -> Arc<[String]> {
    Arc::from(
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<String>>(),
    )
}

#[tokio::test]
async fn a_held_permit_keeps_the_next_check_waiting() {
    let pool = HeuristicsPool::new(1, check_timeout());
    let held = Arc::clone(&pool.permits)
        .try_acquire_owned()
        .expect("the pool has one free permit");
    let shared = keywords(&["ignore prior and start over"]);
    let blocked = timeout(
        Duration::from_millis(20),
        pool.check(
            Arc::clone(&shared),
            "ignore prior and start over".to_owned(),
            threshold(),
        ),
    )
    .await;
    blocked.expect_err("the held permit keeps the check waiting");
    drop(held);
    let outcome = timeout(
        Duration::from_secs(1),
        pool.check(
            Arc::clone(&shared),
            "ignore prior and start over".to_owned(),
            threshold(),
        ),
    )
    .await
    .expect("the released permit lets the check run");
    assert!(outcome.expect("the check completes"));
}

#[tokio::test]
async fn a_pool_holds_one_permit_per_configured_thread() {
    let pool = HeuristicsPool::new(2, check_timeout());
    assert_eq!(pool.permits.available_permits(), 2);
    let first = Arc::clone(&pool.permits)
        .try_acquire_owned()
        .expect("a first permit");
    let second = Arc::clone(&pool.permits)
        .try_acquire_owned()
        .expect("a second permit");
    assert!(
        Arc::clone(&pool.permits).try_acquire_owned().is_err(),
        "a third check must wait"
    );
    drop(first);
    drop(second);
    assert_eq!(pool.permits.available_permits(), 2);
}

#[tokio::test]
async fn a_check_reports_hits_and_clean_prompts() {
    let pool = HeuristicsPool::new(1, check_timeout());
    let shared = keywords(&["ignore prior and start over"]);
    assert!(
        pool.check(
            Arc::clone(&shared),
            "please ignore prior and start over".to_owned(),
            threshold(),
        )
        .await
        .expect("the check completes")
    );
    assert!(
        !pool
            .check(
                Arc::clone(&shared),
                "please summarize the document".to_owned(),
                threshold(),
            )
            .await
            .expect("the check completes")
    );
}

#[test]
fn a_huge_thread_count_is_clamped_to_the_semaphore_limit() {
    let pool = HeuristicsPool::new(usize::try_from(i64::MAX).unwrap(), check_timeout());
    assert_eq!(pool.permits.available_permits(), Semaphore::MAX_PERMITS);
}

#[tokio::test]
async fn a_closed_pool_fails_the_check() {
    let pool = HeuristicsPool::new(1, check_timeout());
    Arc::clone(&pool.permits).close();
    let error = pool
        .check(
            keywords(&["ignore prior and start over"]),
            "ignore prior and start over".to_owned(),
            threshold(),
        )
        .await
        .expect_err("a closed semaphore admits nothing");
    assert!(matches!(error, PromptInjectionError::HeuristicsClosed));
}

#[tokio::test]
async fn a_check_queued_behind_a_held_permit_times_out() {
    let pool = HeuristicsPool::new(1, Duration::from_millis(50));
    let held = Arc::clone(&pool.permits)
        .try_acquire_owned()
        .expect("a held permit");
    let shared = keywords(&["ignore prior and start over"]);
    let error = timeout(
        Duration::from_secs(1),
        pool.check(
            Arc::clone(&shared),
            "ignore prior and start over".to_owned(),
            threshold(),
        ),
    )
    .await
    .expect("the check gives up at its own deadline")
    .expect_err("a queued check times out");
    assert!(matches!(error, PromptInjectionError::HeuristicsTimeout));
    drop(held);
}

#[tokio::test]
async fn a_long_scan_stops_at_the_deadline() {
    let pool = HeuristicsPool::new(1, Duration::from_millis(20));
    let shared = keywords(&["ignore prior and start over"]);
    let long_clean = "x".repeat(100_000);
    let error = pool
        .check(Arc::clone(&shared), long_clean, threshold())
        .await
        .expect_err("a long scan times out");
    assert!(matches!(error, PromptInjectionError::HeuristicsTimeout));
}
