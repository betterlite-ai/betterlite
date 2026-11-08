#![cfg(test)]

use std::future::Future;
use std::iter;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use rstest::rstest;
use tokio::time::Instant;

use super::error::{AcquireError, BoundsError};
use super::{AdaptiveConcurrencyLimiter, MAX_WAITERS, PutSample};

fn limiter(initial: u64, floor: u64, ceiling: u64) -> AdaptiveConcurrencyLimiter {
    AdaptiveConcurrencyLimiter::new(initial, floor, ceiling).unwrap()
}

fn deadline_after(millis: u64) -> Instant {
    Instant::now()
        .checked_add(Duration::from_millis(millis))
        .unwrap()
}

fn poll_once<Output>(future: Pin<&mut dyn Future<Output = Output>>) -> Poll<Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[rstest]
#[case::initial_below_floor_is_clamped(limiter(0, 2, 5), &[], 2, 0)]
#[case::initial_above_ceiling_is_clamped(limiter(9, 2, 5), &[], 5, 0)]
#[case::initial_inside_bounds_is_kept(limiter(3, 2, 5), &[], 3, 0)]
#[case::one_clean_sample_below_the_streak_keeps_the_limit(limiter(2, 1, 8), &[false], 2, 1)]
#[case::clean_streak_at_the_limit_adds_one(limiter(2, 1, 8), &[false, false], 3, 0)]
#[case::clean_streak_after_growth_counts_again(limiter(2, 1, 8), &[false, false, false], 3, 1)]
#[case::throttle_halves_the_limit(limiter(8, 2, 8), &[true], 4, 0)]
#[case::throttled_halving_stops_at_the_floor(limiter(8, 2, 8), &[true, true, true], 2, 0)]
#[case::throttle_from_three_reaches_one(limiter(3, 1, 8), &[true], 1, 0)]
#[case::clean_sample_after_a_throttle_restarts_the_streak(limiter(8, 2, 8), &[true, false], 4, 1)]
#[case::fixed_width_never_grows(limiter(5, 3, 3), &[false, false, false, false], 3, 4)]
#[case::fixed_width_throttle_stays_at_the_floor(limiter(5, 3, 3), &[true], 3, 0)]
#[case::ceiling_stops_growth(
    limiter(1, 1, 4),
    &[
        false, false, false, false, false, false, false, false, false, false,
    ],
    4,
    4
)]
fn adaptive_concurrency_behaviors(
    #[case] limiter: AdaptiveConcurrencyLimiter,
    #[case] samples: &[bool],
    #[case] limit: u64,
    #[case] clean_streak: u64,
) {
    for throttled in samples {
        limiter.record(PutSample {
            throttled: *throttled,
        });
    }
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.limit, limit);
    assert_eq!(snapshot.clean_streak, clean_streak);
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.waiters, 0);
}

#[rstest]
#[case::floor_zero_is_rejected(1, 0, 2, Err(BoundsError::Invalid { floor: 0, ceiling: 2 }))]
#[case::floor_above_ceiling_is_rejected(1, 4, 3, Err(BoundsError::Invalid { floor: 4, ceiling: 3 }))]
#[case::floor_one_ceiling_one_is_valid(1, 1, 1, Ok(1))]
#[case::initial_exactly_at_the_floor(2, 2, 5, Ok(2))]
#[case::initial_exactly_at_the_ceiling(5, 2, 5, Ok(5))]
#[case::initial_one_below_the_floor(1, 2, 5, Ok(2))]
#[case::initial_one_above_the_ceiling(6, 2, 5, Ok(5))]
#[case::zero_initial(0, 1, 5, Ok(1))]
#[case::largest_initial(u64::MAX, 1, 5, Ok(5))]
fn adaptive_concurrency_boundaries(
    #[case] initial: u64,
    #[case] floor: u64,
    #[case] ceiling: u64,
    #[case] expected: Result<u64, BoundsError>,
) {
    let built = AdaptiveConcurrencyLimiter::new(initial, floor, ceiling)
        .map(|limiter| limiter.snapshot().limit);
    assert_eq!(built, expected);
}

#[tokio::test]
async fn release_grants_the_oldest_waiter_first() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let mut first = Box::pin(limiter.acquire(deadline_after(60_000)));
    let mut second = Box::pin(limiter.acquire(deadline_after(60_000)));
    assert!(poll_once(first.as_mut()).is_pending());
    assert!(poll_once(second.as_mut()).is_pending());
    drop(held);
    assert_eq!(limiter.snapshot().waiters, 1);
    let Poll::Ready(Ok(first_permit)) = poll_once(first.as_mut()) else {
        panic!("the oldest waiter is granted first")
    };
    assert!(poll_once(second.as_mut()).is_pending());
    drop(first_permit);
    let Poll::Ready(Ok(_second_permit)) = poll_once(second.as_mut()) else {
        panic!("the next waiter is granted after the first releases")
    };
}

#[tokio::test]
async fn a_clean_record_at_the_limit_grants_a_waiter() {
    let limiter = limiter(1, 1, 8);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let mut waiting = Box::pin(limiter.acquire(deadline_after(60_000)));
    assert!(poll_once(waiting.as_mut()).is_pending());
    limiter.record(PutSample { throttled: false });
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.limit, 2);
    assert_eq!(snapshot.in_flight, 2);
    assert_eq!(snapshot.waiters, 0);
    assert_eq!(snapshot.clean_streak, 0);
    let Poll::Ready(Ok(_waiting_permit)) = poll_once(waiting.as_mut()) else {
        panic!("growth grants the queued waiter")
    };
    drop(held);
}

#[tokio::test]
async fn an_expired_deadline_grants_a_free_permit() {
    let limiter = limiter(1, 1, 1);
    let Ok(permit) = limiter.acquire(Instant::now()).await else {
        panic!("an expired deadline still takes a free permit")
    };
    assert_eq!(limiter.snapshot().in_flight, 1);
    drop(permit);
}

#[tokio::test]
async fn an_expired_deadline_does_not_enqueue() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let denied = limiter.acquire(Instant::now()).await;
    assert!(matches!(denied, Err(AcquireError::DeadlineExceeded)));
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.waiters, 0);
    assert_eq!(snapshot.in_flight, 1);
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn a_queued_waiter_deadline_expires() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let denied = limiter.acquire(deadline_after(5)).await;
    assert!(matches!(denied, Err(AcquireError::DeadlineExceeded)));
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.waiters, 0);
    assert_eq!(snapshot.in_flight, 1);
    drop(held);
}

#[tokio::test]
async fn the_waiter_cap_returns_overloaded_before_the_1025th_waiter() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let mut queued: Vec<_> =
        iter::repeat_with(|| Box::pin(limiter.acquire(deadline_after(60_000))))
            .take(MAX_WAITERS)
            .collect();
    assert!(
        queued
            .iter_mut()
            .all(|waiting| poll_once(waiting.as_mut()).is_pending())
    );
    let mut overflow = Box::pin(limiter.acquire(deadline_after(60_000)));
    assert!(matches!(
        poll_once(overflow.as_mut()),
        Poll::Ready(Err(AcquireError::Overloaded))
    ));
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.waiters, MAX_WAITERS);
    assert_eq!(snapshot.in_flight, 1);
    drop(held);
}

#[tokio::test]
async fn cancelling_a_queued_waiter_releases_nothing() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let mut queued = Box::pin(limiter.acquire(deadline_after(60_000)));
    assert!(poll_once(queued.as_mut()).is_pending());
    drop(queued);
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.waiters, 0);
    assert_eq!(snapshot.in_flight, 1);
    drop(held);
    assert_eq!(limiter.snapshot().in_flight, 0);
}

#[tokio::test]
async fn cancelling_a_granted_waiter_returns_the_slot_exactly_once() {
    let limiter = limiter(1, 1, 1);
    let held = limiter.acquire(deadline_after(60_000)).await.unwrap();
    let mut granted = Box::pin(limiter.acquire(deadline_after(60_000)));
    assert!(poll_once(granted.as_mut()).is_pending());
    drop(held);
    assert_eq!(limiter.snapshot().in_flight, 1);
    drop(granted);
    let snapshot = limiter.snapshot();
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.waiters, 0);
    let again = limiter.acquire(deadline_after(60_000)).await.unwrap();
    assert_eq!(limiter.snapshot().in_flight, 1);
    drop(again);
    assert_eq!(limiter.snapshot().in_flight, 0);
}
