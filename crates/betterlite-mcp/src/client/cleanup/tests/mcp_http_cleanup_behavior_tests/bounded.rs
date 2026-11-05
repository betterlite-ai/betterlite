use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::super::super::bounded::{CleanupError, run_bounded_cleanup};
use super::{FakeError, deadline_in, seconds};

#[tokio::test(start_paused = true)]
async fn returns_the_operation_result() {
    let tracker = TaskTracker::new();
    let result = run_bounded_cleanup(&tracker, &CancellationToken::new(), deadline_in(5), async {
        Ok::<u8, FakeError>(7)
    })
    .await;
    assert_eq!(result.unwrap(), 7_u8);
}

#[tokio::test(start_paused = true)]
async fn propagates_the_operation_error() {
    let tracker = TaskTracker::new();
    let result = run_bounded_cleanup(&tracker, &CancellationToken::new(), deadline_in(5), async {
        Err::<u8, _>(FakeError::Boom("close"))
    })
    .await;
    assert!(matches!(
        result,
        Err(CleanupError::Operation(FakeError::Boom("close")))
    ));
}

#[tokio::test(start_paused = true)]
async fn stops_the_operation_at_the_deadline() {
    let tracker = TaskTracker::new();
    let started = Instant::now();
    let result = run_bounded_cleanup(
        &tracker,
        &CancellationToken::new(),
        started.checked_add(seconds(5)).unwrap(),
        async {
            sleep(seconds(60)).await;
            Ok::<_, FakeError>(())
        },
    )
    .await;
    assert!(matches!(result, Err(CleanupError::TimedOut(_))));
    assert_eq!(started.elapsed(), seconds(5));
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_waits_for_the_cleanup_then_wins() {
    let tracker = TaskTracker::new();
    let cancelled = CancellationToken::new();
    let finished = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&finished);
    cancelled.cancel();
    let result = run_bounded_cleanup(&tracker, &cancelled, deadline_in(5), async move {
        sleep(seconds(2)).await;
        marker.store(true, Ordering::SeqCst);
        Ok::<u8, FakeError>(1)
    })
    .await;
    assert!(matches!(result, Err(CleanupError::Cancelled)));
    assert!(finished.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_wins_over_a_failed_cleanup() {
    let tracker = TaskTracker::new();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let result = run_bounded_cleanup(&tracker, &cancelled, deadline_in(5), async {
        sleep(seconds(1)).await;
        Err::<u8, _>(FakeError::Boom("late"))
    })
    .await;
    assert!(matches!(result, Err(CleanupError::Cancelled)));
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_the_wait_does_not_abandon_the_cleanup() {
    let tracker = TaskTracker::new();
    let cancelled = CancellationToken::new();
    let finished = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&finished);
    let trigger = cancelled.clone();
    tracker.spawn(async move {
        sleep(seconds(1)).await;
        trigger.cancel();
    });
    let result = run_bounded_cleanup(&tracker, &cancelled, deadline_in(5), async move {
        sleep(seconds(3)).await;
        marker.store(true, Ordering::SeqCst);
        Ok::<_, FakeError>(())
    })
    .await;
    assert!(matches!(result, Err(CleanupError::Cancelled)));
    assert!(finished.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_caller_leaves_the_cleanup_running_in_the_tracker() {
    let tracker = TaskTracker::new();
    let finished = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&finished);
    let never_cancelled = CancellationToken::new();
    let waiting = run_bounded_cleanup(&tracker, &never_cancelled, deadline_in(5), async move {
        sleep(seconds(2)).await;
        marker.store(true, Ordering::SeqCst);
        Ok::<_, FakeError>(())
    });
    let abandoned = timeout(seconds(1), waiting).await;
    abandoned.unwrap_err();
    tracker.close();
    tracker.wait().await;
    assert!(finished.load(Ordering::SeqCst));
}
