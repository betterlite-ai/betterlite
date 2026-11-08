#![cfg(test)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_router::adaptive_concurrency::error::AcquireError;
use betterlite_router::adaptive_concurrency::{AdaptiveConcurrencyLimiter, PutSample};
use tokio::time::advance;

use super::IntegrationConcurrency;

fn poll_once<Output>(future: Pin<&mut dyn Future<Output = Output>>) -> Poll<Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn integration(
    initial: u64,
    floor: u64,
    ceiling: u64,
    wait_millis: u64,
) -> (IntegrationConcurrency, Arc<AdaptiveConcurrencyLimiter>) {
    let limiter = Arc::new(AdaptiveConcurrencyLimiter::new(initial, floor, ceiling).unwrap());
    (
        IntegrationConcurrency {
            limiter: Arc::clone(&limiter),
            wait_timeout: Duration::from_millis(wait_millis),
        },
        limiter,
    )
}

#[tokio::test]
async fn a_free_permit_is_granted_without_waiting() {
    let (integration, limiter) = integration(1, 1, 1, 5);
    let permit = integration.permit().await.unwrap();
    assert_eq!(limiter.snapshot().in_flight, 1);
    drop(permit);
    assert_eq!(limiter.snapshot().in_flight, 0);
}

#[tokio::test(start_paused = true)]
async fn a_wait_past_the_callback_timeout_is_denied() {
    let (integration, limiter) = integration(1, 1, 1, 5);
    let held = integration.permit().await.unwrap();
    let mut waiting = Box::pin(integration.permit());
    assert!(matches!(poll_once(waiting.as_mut()), Poll::Pending));
    assert_eq!(limiter.snapshot().waiters, 1);
    advance(Duration::from_millis(4)).await;
    assert!(matches!(poll_once(waiting.as_mut()), Poll::Pending));
    assert_eq!(limiter.snapshot().waiters, 1);
    advance(Duration::from_millis(2)).await;
    assert!(matches!(
        poll_once(waiting.as_mut()),
        Poll::Ready(Err(AcquireError::DeadlineExceeded))
    ));
    assert_eq!(limiter.snapshot().waiters, 0);
    drop(held);
    assert_eq!(limiter.snapshot().in_flight, 0);
}

#[tokio::test]
async fn recording_a_throttled_sample_halves_the_limit() {
    let (integration, limiter) = integration(8, 2, 8, 5);
    let limit = integration.record(PutSample { throttled: true });
    assert_eq!(limit, 4);
    assert_eq!(limiter.snapshot().limit, 4);
}

#[tokio::test]
async fn recording_a_clean_sample_grows_the_limit_at_the_streak() {
    let (integration, limiter) = integration(2, 1, 8, 5);
    let first = integration.record(PutSample { throttled: false });
    assert_eq!(first, 2);
    let grown = integration.record(PutSample { throttled: false });
    assert_eq!(grown, 3);
    assert_eq!(limiter.snapshot().limit, 3);
}
