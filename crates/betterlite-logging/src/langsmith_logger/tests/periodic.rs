use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::{advance, timeout};

use super::support::{FakeDelivery, Options, dynamic, event, harness};
use crate::callback_logger::CallbackLogger as _;

fn global_key() -> [(&'static str, &'static str); 1] {
    [("LANGSMITH_API_KEY", "global-key")]
}

#[tokio::test(start_paused = true)]
async fn the_periodic_flush_sends_after_the_interval() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let (delivery, mut receiver) = FakeDelivery::answering(200).subscribed();
    let built = harness(&options, delivery);
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is queued");
    assert!(built.delivery.posts().is_empty());
    yield_now().await;
    advance(Duration::from_secs(5)).await;
    let posted = timeout(Duration::from_secs(50), receiver.recv()).await;
    assert!(posted.expect("the periodic flush runs").is_some());
    assert_eq!(built.delivery.posts().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_the_periodic_flush() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is queued");
    yield_now().await;
    built.shutdown.cancel();
    advance(Duration::from_secs(5)).await;
    built.tasks.close();
    built.tasks.wait().await;
    assert!(built.delivery.posts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_shared_flush_interval_variable_sets_the_period() {
    let options = Options {
        environment: &[
            ("LANGSMITH_API_KEY", "global-key"),
            ("DEFAULT_FLUSH_INTERVAL_SECONDS", "9"),
        ],
        batch_size: Some(5),
        draws: &[0.0_f64],
        redact: false,
    };
    let (delivery, mut receiver) = FakeDelivery::answering(200).subscribed();
    let built = harness(&options, delivery);
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is queued");
    yield_now().await;
    advance(Duration::from_secs(5)).await;
    assert!(built.delivery.posts().is_empty());
    advance(Duration::from_secs(4)).await;
    let posted = timeout(Duration::from_secs(50), receiver.recv()).await;
    assert!(posted.expect("the periodic flush runs").is_some());
}
