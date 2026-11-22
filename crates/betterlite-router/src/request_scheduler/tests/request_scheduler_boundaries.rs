use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use betterlite_config::sections::request_scheduler::Settings;
use betterlite_error::envelope::WireField;
use rstest::rstest;

use crate::request_scheduler::error::SchedulerError;
use crate::request_scheduler::flow::Priority;
use crate::request_scheduler::queue::MAX_PENDING_REQUESTS;
use crate::request_scheduler::service::RequestScheduler;
use crate::routing_value_store::{RoutingValueStore, StoreSettings};

use super::{
    MODEL, ScriptedCall, entries, fill_queue, item, local_store, plan, scheduler,
    shared_scheduler_with,
};

// add_request refuses the request past the bound and keeps the queue where it was
#[rstest]
#[case::the_last_free_slot(MAX_PENDING_REQUESTS - 1, true)]
#[case::one_past_the_bound(MAX_PENDING_REQUESTS, false)]
#[tokio::test]
async fn the_queue_holds_at_most_1024_requests(#[case] pending: usize, #[case] accepted: bool) {
    let service = scheduler(Settings::default());
    fill_queue(&service, pending).await;
    let added = service.add_request(&item(0, "last", MODEL)).await;
    if accepted {
        added.expect("the request fits in the last free slot");
        assert_eq!(
            service
                .queue(MODEL)
                .await
                .expect("the queue reads")
                .value
                .len(),
            MAX_PENDING_REQUESTS
        );
        return;
    }
    let error = added.expect_err("the queue is full");
    assert!(matches!(
        error,
        SchedulerError::QueueFull {
            limit: MAX_PENDING_REQUESTS,
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.code(), None);
    assert_eq!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .len(),
        MAX_PENDING_REQUESTS
    );
}

#[rstest]
#[case::empty("")]
#[case::the_member_separator("a:b")]
#[tokio::test]
async fn a_request_id_the_queue_cannot_encode_is_refused(#[case] request_id: &str) {
    let service = scheduler(Settings::default());
    let error = service
        .add_request(&item(0, request_id, MODEL))
        .await
        .expect_err("the request id cannot be encoded");
    assert!(matches!(error, SchedulerError::InvalidRequestId { .. }));
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(error.code(), None);
    assert!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .is_empty()
    );
}

// the bound is 128 bytes: the request id one byte over it is refused
#[tokio::test]
async fn a_request_id_stops_at_128_bytes() {
    let service = scheduler(Settings::default());
    let fitting = "a".repeat(128);
    service
        .add_request(&item(0, &fitting, MODEL))
        .await
        .expect("128 bytes fit");
    let over = "a".repeat(129);
    let error = service
        .add_request(&item(0, &over, MODEL))
        .await
        .expect_err("129 bytes do not fit");
    assert!(matches!(error, SchedulerError::InvalidRequestId { .. }));
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, fitting.as_str())])
    );
}

// the injected source passes the same encoder bound as a queued request id
#[test]
fn an_injected_request_id_past_128_bytes_is_refused() {
    let service = RequestScheduler::new(
        local_store(),
        None,
        Settings::default(),
        Arc::new(|| "a".repeat(129)),
    );
    let error = service.next_request_id().expect_err("129 bytes do not fit");
    assert!(matches!(
        error,
        SchedulerError::InvalidRequestId { ref id } if id.len() == 129
    ));
    assert_eq!(error.status_code(), Some(400));
}

// the routing deadline stops the wait and takes the entry out, without reaching the provider
#[tokio::test(start_paused = true)]
async fn a_waiting_request_times_out_at_the_deadline() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    let call = ScriptedCall::answered(0);
    let error = service
        .schedule(
            &item(Priority::Low.value(), "waiting", MODEL),
            &call,
            &plan(0.2),
        )
        .await
        .expect_err("the deadline passes before the head does");
    assert!(matches!(
        error,
        SchedulerError::TimedOut {
            ref model,
            ref request_id,
        } if model == MODEL && request_id == "waiting"
    ));
    assert_eq!(error.status_code(), Some(408));
    assert_eq!(error.code(), None);
    assert_eq!(call.calls(), 0);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "head")])
    );
}

// a zero timeout is already expired when the loop starts, so the request never polls
#[tokio::test(start_paused = true)]
async fn a_zero_timeout_expires_before_the_first_poll() {
    let service = scheduler(Settings::default());
    let call = ScriptedCall::answered(0);
    let error = service
        .schedule(&item(0, "immediate", MODEL), &call, &plan(0.0))
        .await
        .expect_err("the deadline is the start");
    assert!(matches!(error, SchedulerError::TimedOut { .. }));
    assert_eq!(call.calls(), 0);
    assert!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .is_empty()
    );
}

// a caller that cancelled before the loop starts leaves no entry behind
#[tokio::test(start_paused = true)]
async fn a_cancelled_request_leaves_the_queue_before_the_wait() {
    let service = scheduler(Settings::default());
    let call = ScriptedCall::answered(0);
    let timeouts = plan(5.0);
    timeouts.cancellation.cancel();
    let error = service
        .schedule(
            &item(Priority::Medium.value(), "cancelled", MODEL),
            &call,
            &timeouts,
        )
        .await
        .expect_err("the caller went away");
    assert!(matches!(
        error,
        SchedulerError::Cancelled {
            ref request_id,
            ref model,
        } if request_id == "cancelled" && model == MODEL
    ));
    assert_eq!(error.status_code(), None);
    assert_eq!(call.calls(), 0);
    assert!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .is_empty()
    );
}

// a cancellation during the poll sleep leaves no entry behind either
#[tokio::test(start_paused = true)]
async fn a_cancelled_wait_leaves_the_queue() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    let timeouts = plan(5.0);
    let call = ScriptedCall::cancelling(0, timeouts.cancellation.clone());
    let error = service
        .schedule(
            &item(Priority::Low.value(), "waiting", MODEL),
            &call,
            &timeouts,
        )
        .await
        .expect_err("the caller went away while the request waited");
    assert!(matches!(error, SchedulerError::Cancelled { .. }));
    assert_eq!(call.calls(), 0);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "head")])
    );
}

// the provider's status and code survive, and the error carries the request's priority
#[tokio::test(start_paused = true)]
async fn a_provider_failure_keeps_the_status_code_and_the_stamped_priority() {
    // a configured default_priority is what makes the pin's inner call schedule the same request again
    let service = scheduler(Settings {
        polling_interval: 0.03_f64,
        default_priority: Some(Priority::Medium.value()),
        local_ttl_seconds: 5.0_f64,
    });
    let call = ScriptedCall::failed(0, 429, Some("429"));
    let error = service
        .schedule(
            &item(Priority::Low.value(), "failed", MODEL),
            &call,
            &plan(0.5),
        )
        .await
        .expect_err("the provider refused the call");
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.code(), Some("429".to_owned()));
    assert_eq!(error.priority(), Some(Priority::Low.value()));
    match error {
        SchedulerError::Provider {
            error: cause,
            priority,
        } => {
            let metadata = cause.metadata();
            assert_eq!(metadata.status_code, 429);
            assert_eq!(
                metadata.error_type,
                WireField::Text("invalid_request_error".to_owned())
            );
            assert_eq!(metadata.param, WireField::Text("model".to_owned()));
            assert_eq!(priority, Priority::Low.value());
        }
        other => panic!("expected a provider failure, got {other:?}"),
    }
    assert_eq!(call.calls(), 1);
    assert!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .is_empty()
    );
}

// the healthy-deployment count decides the poll at 0 and at 1
#[tokio::test]
async fn the_healthy_deployment_count_selects_the_head() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    service
        .add_request(&item(Priority::Low.value(), "late", MODEL))
        .await
        .expect("the late request queues");
    let no_healthy = service
        .poll("late", MODEL, 0)
        .await
        .expect("the poll answers");
    assert!(!no_healthy.value);
    let one_healthy = service
        .poll("late", MODEL, 1)
        .await
        .expect("the poll answers");
    assert!(one_healthy.value);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "head"), (255, "late")])
    );
}

#[tokio::test]
async fn the_named_priorities_are_the_pinned_bytes() {
    assert_eq!(Priority::High.value(), 0);
    assert_eq!(Priority::Medium.value(), 128);
    assert_eq!(Priority::Low.value(), 255);
}

// DEFAULT_IN_MEMORY_TTL: the local copy beside the shared tier lives past 2 seconds and is gone at 6
#[tokio::test]
async fn the_local_queue_copy_expires_at_the_shared_ttl() {
    let clock = Arc::new(AtomicU64::new(0));
    let reading = Arc::clone(&clock);
    let local = RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(move || Duration::from_millis(reading.load(Ordering::SeqCst)).as_secs_f64()),
    )
    .expect("the local store builds");
    let (service, server) = shared_scheduler_with(local).await;
    service
        .add_request(&item(0, "first", MODEL))
        .await
        .expect("the request queues");
    server.set_down(true);
    clock.store(2_000, Ordering::SeqCst);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "first")]),
        "the copy is alive inside the ttl"
    );
    clock.store(6_000, Ordering::SeqCst);
    assert!(
        service
            .queue(MODEL)
            .await
            .expect("the queue reads")
            .value
            .is_empty(),
        "the copy is gone past the ttl"
    );
}

// a local store whose clock turns non-finite on demand, so its next write fails while earlier ones worked
fn breakable_local_store(broken: &Arc<AtomicBool>) -> RoutingValueStore {
    let reading = Arc::clone(broken);
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(move || {
            if reading.load(Ordering::SeqCst) {
                f64::NAN
            } else {
                0.0_f64
            }
        }),
    )
    .expect("the local store builds")
}

// a successful call whose queue entry cannot be removed answers its response instead of the cleanup error
#[tokio::test(start_paused = true)]
async fn a_completed_request_keeps_its_response_when_the_cleanup_fails() {
    let broken = Arc::new(AtomicBool::new(false));
    let (service, _server) = shared_scheduler_with(breakable_local_store(&broken)).await;
    let call = ScriptedCall::answered_flipping(0, Arc::clone(&broken));
    let outcome = service
        .schedule(&item(0, "completed", MODEL), &call, &plan(0.5))
        .await
        .expect("the cleanup failure does not replace the response");
    assert!(outcome.prioritized);
    assert_eq!(call.calls(), 1);
}

// the deadline reports its timeout even when the queue entry cannot be removed
#[tokio::test(start_paused = true)]
async fn a_timed_out_request_answers_its_timeout_when_the_cleanup_fails() {
    let broken = Arc::new(AtomicBool::new(false));
    let (service, _server) = shared_scheduler_with(breakable_local_store(&broken)).await;
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    let call = ScriptedCall::answered(0);
    let flipping = Arc::clone(&broken);
    let flip = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        flipping.store(true, Ordering::SeqCst);
    });
    let error = service
        .schedule(
            &item(Priority::Low.value(), "late", MODEL),
            &call,
            &plan(0.5),
        )
        .await
        .expect_err("the deadline expires");
    flip.await.expect("the clock flips");
    assert!(matches!(error, SchedulerError::TimedOut { .. }));
    assert_eq!(call.calls(), 0);
}
