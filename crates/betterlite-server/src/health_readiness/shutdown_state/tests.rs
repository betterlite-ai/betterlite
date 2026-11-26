#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rstest::rstest;
use tokio::time::{Instant, sleep};

use super::{InFlightCounter, ShutdownState};
use crate::health_readiness::tests::support::{FakeInFlight, capture_logs};

fn state(in_flight: &Arc<FakeInFlight>, timeout: Duration) -> ShutdownState {
    let counter: Arc<dyn InFlightCounter> = Arc::<FakeInFlight>::clone(in_flight);
    ShutdownState::new(counter, timeout)
}

#[test]
fn shutdown_flag_is_false_until_started() {
    let in_flight = Arc::new(FakeInFlight::new(0));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    assert!(!shutdown.is_shutting_down());
    shutdown.start_shutdown();
    assert!(shutdown.is_shutting_down());
}

#[test]
fn shutdown_start_is_idempotent_and_logs_once() {
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let in_flight = Arc::new(FakeInFlight::new(3));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    shutdown.start_shutdown();
    shutdown.start_shutdown();
    let contents = logs.contents();
    assert!(contents.contains("graceful_shutdown_started in_flight_requests=3"));
    assert_eq!(contents.matches("graceful_shutdown_started").count(), 1);
}

#[tokio::test(start_paused = true)]
async fn drain_wait_returns_the_drained_count_once_requests_finish() {
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let in_flight = Arc::new(FakeInFlight::new(3));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let counter = Arc::clone(&in_flight);
    let (drained, ()) = tokio::join!(shutdown.wait_for_drain(None, false), async move {
        sleep(Duration::from_millis(300)).await;
        counter.count.store(0, Ordering::Release);
    });
    assert_eq!(drained, 3);
    assert!(
        logs.contents()
            .contains("graceful_shutdown_complete drained_requests=3")
    );
}

#[tokio::test(start_paused = true)]
async fn drain_wait_exclude_self_treats_its_own_request_as_drained() {
    let in_flight = Arc::new(FakeInFlight::new(2));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let counter = Arc::clone(&in_flight);
    let (drained, ()) = tokio::join!(shutdown.wait_for_drain(None, true), async move {
        sleep(Duration::from_millis(200)).await;
        counter.count.store(1, Ordering::Release);
    });
    assert_eq!(drained, 1);
}

#[tokio::test(start_paused = true)]
async fn drain_wait_second_call_returns_zero_at_once() {
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let in_flight = Arc::new(FakeInFlight::new(1));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let counter = Arc::clone(&in_flight);
    let (first, ()) = tokio::join!(shutdown.wait_for_drain(None, false), async move {
        sleep(Duration::from_millis(100)).await;
        counter.count.store(0, Ordering::Release);
    });
    assert_eq!(first, 1);
    in_flight.count.store(5, Ordering::Release);
    let started = Instant::now();
    let second = shutdown.wait_for_drain(None, false).await;
    assert_eq!(second, 0);
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(
        logs.contents()
            .matches("graceful_shutdown_complete")
            .count(),
        1
    );
}

#[rstest]
#[case::waiting_for_every_request(false, 4)]
#[case::excluding_its_own_request(true, 3)]
#[tokio::test(start_paused = true)]
async fn drain_wait_with_a_zero_timeout_returns_the_outstanding_count(
    #[case] exclude_self: bool,
    #[case] expected: u64,
) {
    let in_flight = Arc::new(FakeInFlight::new(4));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let drained = shutdown
        .wait_for_drain(Some(Duration::ZERO), exclude_self)
        .await;
    assert_eq!(drained, expected);
}

#[tokio::test(start_paused = true)]
async fn drain_wait_with_a_zero_timeout_never_goes_below_zero() {
    let in_flight = Arc::new(FakeInFlight::new(0));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let drained = shutdown.wait_for_drain(Some(Duration::ZERO), true).await;
    assert_eq!(drained, 0);
}

#[tokio::test(start_paused = true)]
async fn drain_wait_never_returns_a_negative_count() {
    let in_flight = Arc::new(FakeInFlight::new(3));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let drained = shutdown.poll_drain(1, 0, Duration::from_secs(1)).await;
    assert_eq!(drained, 0);
}

#[tokio::test(start_paused = true)]
async fn drain_wait_times_out_and_returns_what_drained() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let in_flight = Arc::new(FakeInFlight::new(3));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let counter = Arc::clone(&in_flight);
    let (drained, ()) = tokio::join!(
        shutdown.wait_for_drain(Some(Duration::from_secs(1)), false),
        async move {
            sleep(Duration::from_millis(300)).await;
            counter.count.store(2, Ordering::Release);
        }
    );
    assert_eq!(drained, 1);
    let contents = logs.contents();
    assert!(contents.contains("graceful_shutdown_timeout in_flight_requests=2"));
    assert!(contents.contains("elapsed_s=1.00"));
    assert!(contents.contains("timeout_s=1.0"));
    assert!(contents.contains("proceeding with teardown"));
}

#[tokio::test(start_paused = true)]
async fn drain_wait_logs_every_five_seconds() {
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let in_flight = Arc::new(FakeInFlight::new(1));
    let shutdown = state(&in_flight, Duration::from_secs(30));
    let counter = Arc::clone(&in_flight);
    let (drained, ()) = tokio::join!(
        shutdown.wait_for_drain(Some(Duration::from_secs(11)), false),
        async move {
            sleep(Duration::from_millis(10_500)).await;
            counter.count.store(0, Ordering::Release);
        }
    );
    assert_eq!(drained, 1);
    let contents = logs.contents();
    assert_eq!(
        contents
            .matches("drain_waiting in_flight_requests=1")
            .count(),
        2
    );
}
