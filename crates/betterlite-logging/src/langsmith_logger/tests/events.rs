use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use tokio::task::yield_now;
use tokio::time::timeout;

use super::support::{FakeDelivery, Options, dynamic, event, harness, text};
use crate::callback_logger::CallbackLogger as _;

fn global_key() -> [(&'static str, &'static str); 1] {
    [("LANGSMITH_API_KEY", "global-key")]
}

#[tokio::test]
async fn a_sampled_out_event_never_queues() {
    let options = Options {
        environment: &[("LANGSMITH_SAMPLING_RATE", "0")],
        batch_size: Some(1),
        draws: &[0.5_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let (captured, guard) = super::support::capture_logs();
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("a skipped event is not an error");
    drop(guard);
    assert!(built.logger.queue.pending().is_empty());
    assert!(built.delivery.posts().is_empty());
    assert!(captured.contents().contains("Skipping Langsmith logging"));
    assert!(captured.contents().contains("Sampling rate=0"));
}

#[tokio::test]
async fn a_draw_equal_to_the_rate_is_not_sampled_out() {
    let options = Options {
        environment: &[("LANGSMITH_SAMPLING_RATE", "0")],
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    assert_eq!(built.logger.queue.pending().len(), 1);
    assert!(built.delivery.posts().is_empty());
}

#[tokio::test]
async fn a_draw_below_the_rate_queues_the_run() {
    let options = Options {
        environment: &[("LANGSMITH_SAMPLING_RATE", "1")],
        batch_size: Some(5),
        draws: &[0.25_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    assert_eq!(built.logger.queue.pending().len(), 1);
    assert!(built.delivery.posts().is_empty());
}

#[tokio::test]
async fn the_dynamic_sampling_rate_overrides_the_default() {
    let options = Options {
        environment: &[("LANGSMITH_SAMPLING_RATE", "0")],
        batch_size: Some(5),
        draws: &[0.5_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let requested = dynamic(&[(
        "langsmith_sampling_rate",
        Json::Number(serde_json::Number::from(1_i32)),
    )]);
    built
        .logger
        .async_log_success_event(Arc::new(event(requested)))
        .await
        .expect("the event is logged");
    assert_eq!(built.logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn the_sync_event_spawns_the_flush() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let (delivery, mut receiver) = FakeDelivery::answering(200).subscribed();
    let built = harness(&options, delivery);
    built
        .logger
        .log_success_event(&event(dynamic(&[])))
        .expect("the sync event is recorded");
    let posted = timeout(Duration::from_secs(5), receiver.recv()).await;
    assert!(posted.expect("the spawned flush runs").is_some());
    assert_eq!(built.delivery.posts().len(), 1);
}

#[tokio::test]
async fn the_sync_event_below_the_batch_size_keeps_the_run_queued() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .log_success_event(&event(dynamic(&[])))
        .expect("the sync event is recorded");
    yield_now().await;
    assert_eq!(built.logger.queue.pending().len(), 1);
    assert!(built.delivery.posts().is_empty());
}

#[tokio::test]
async fn a_bad_dynamic_sampling_rate_is_logged_and_swallowed() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let (captured, guard) = super::support::capture_logs();
    let requested = dynamic(&[("langsmith_sampling_rate", text("fast"))]);
    built
        .logger
        .async_log_success_event(Arc::new(event(requested)))
        .await
        .expect("a bad sampling rate is not returned");
    drop(guard);
    assert!(built.logger.queue.pending().is_empty());
    assert!(
        captured
            .contents()
            .contains("Langsmith Layer Error - error logging async success event.")
    );
}

#[tokio::test]
async fn the_failure_event_queues_a_run() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_failure_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the failure event is recorded");
    assert_eq!(built.logger.queue.pending().len(), 1);
    assert!(built.delivery.posts().is_empty());
}
