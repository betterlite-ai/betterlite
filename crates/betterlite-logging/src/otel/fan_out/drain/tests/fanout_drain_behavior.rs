#![cfg(test)]

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use opentelemetry_sdk::trace::SpanProcessor;

use crate::otel::fan_out::drain::DrainPool;
use crate::otel::fan_out::tests::support::{
    RELEASE_SLOTS, Recorder, SignalCloser, blocking_closer, blocking_recorder,
};

const WAIT: Duration = Duration::from_secs(10);

fn signal_closer(fail: bool) -> (Arc<SignalCloser>, Receiver<()>) {
    let (signal, received) = mpsc::sync_channel(RELEASE_SLOTS);
    (Arc::new(SignalCloser { signal, fail }), received)
}

#[test]
fn workers_close_submitted_processors() {
    let pool = DrainPool::new(2, 64);
    let (closer, received) = signal_closer(false);
    pool.submit(closer);
    received
        .recv_timeout(WAIT)
        .expect("a worker closes the processor");
    pool.close(WAIT);
}

#[test]
fn saturation_counts_processors_whose_close_has_not_returned() {
    let pool = DrainPool::new(1, 2);
    let (first, first_entered, first_release) = blocking_closer();
    pool.submit(first);
    first_entered
        .recv_timeout(WAIT)
        .expect("the first close starts");
    assert!(!pool.saturated());

    let (second, second_entered, second_release) = blocking_closer();
    pool.submit(second);
    assert!(pool.saturated());

    first_release.send(()).expect("the first close may return");
    second_entered
        .recv_timeout(WAIT)
        .expect("the second close starts");
    second_release
        .send(())
        .expect("the second close may return");
    pool.close(WAIT);
    assert!(!pool.saturated());
}

#[test]
fn a_processor_submitted_past_close_is_closed_off_the_submitting_thread() {
    let pool = DrainPool::new(1, 64);
    pool.close(Duration::ZERO);
    let (closer, received) = signal_closer(false);
    pool.submit(closer);
    received.recv_timeout(WAIT).expect("a straggler closes it");
}

#[test]
fn close_waits_for_live_workers_until_the_deadline() {
    let pool = DrainPool::new(1, 64);
    let (closer, entered, release) = blocking_closer();
    pool.submit(closer);
    entered.recv_timeout(WAIT).expect("the close starts");
    let started = Instant::now();
    pool.close(Duration::from_millis(200));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(150),
        "close returned in {waited:?}"
    );
    release.send(()).expect("the flush finishes");
}

#[test]
fn close_returns_once_every_worker_has_retired() {
    let pool = DrainPool::new(1, 64);
    let (closer, received) = signal_closer(false);
    pool.submit(closer);
    received
        .recv_timeout(WAIT)
        .expect("the worker closes the processor");
    let started = Instant::now();
    pool.close(WAIT);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn dropping_the_pool_stops_its_worker() {
    let pool = DrainPool::new(1, 8);
    let shared = Arc::clone(&pool.shared);
    assert_eq!(Arc::strong_count(&shared), 3);
    drop(pool);
    let deadline = Instant::now() + WAIT;
    while Arc::strong_count(&shared) > 1 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[test]
fn close_waits_only_for_its_deadline_before_abandoning_a_flush() {
    let pool = DrainPool::new(1, 64);
    let (closer, entered, release) = blocking_closer();
    pool.submit(closer);
    entered.recv_timeout(WAIT).expect("the close starts");
    let started = Instant::now();
    pool.close(Duration::from_millis(50));
    assert!(started.elapsed() < Duration::from_secs(3));
    release.send(()).expect("the flush finishes");
}

#[test]
fn a_failing_close_keeps_the_worker_draining() {
    let pool = DrainPool::new(1, 64);
    let (failing, failing_received) = signal_closer(true);
    let (healthy, healthy_received) = signal_closer(false);
    pool.submit(failing);
    failing_received
        .recv_timeout(WAIT)
        .expect("the failing close runs");
    pool.submit(healthy);
    healthy_received
        .recv_timeout(WAIT)
        .expect("the next close runs");
    pool.close(WAIT);
}

#[test]
fn a_pool_that_could_not_start_a_worker_closes_the_processor_inline() {
    let pool = DrainPool::new(0, 64);
    let recorder = Recorder::new();
    let shared = Arc::clone(&recorder);
    let processor: Arc<dyn SpanProcessor> = shared;
    pool.submit(processor);
    assert_eq!(recorder.shutdown_count(), 1);
}

#[test]
fn close_twice_does_not_wait_for_the_workers_again() {
    let pool = DrainPool::new(2, 64);
    pool.close(Duration::ZERO);
    let started = Instant::now();
    pool.close(Duration::from_secs(10));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_retired_processor_is_closed_exactly_once() {
    let pool = DrainPool::new(1, 64);
    let (closer, _entered, _release) = blocking_recorder();
    let shared = Arc::clone(&closer);
    let processor: Arc<dyn SpanProcessor> = shared;
    pool.submit(processor);
    pool.close(WAIT);
    assert_eq!(closer.shutdown_count(), 1);
}
