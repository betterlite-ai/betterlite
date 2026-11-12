use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_llm_observability::flush::queue::flush_queue;
use crate::datadog_llm_observability::settings::DatadogLlmObsInit;
use crate::datadog_llm_observability::tests::support;

fn threshold_init(batch_size: usize) -> DatadogLlmObsInit {
    let mut init = support::agent_init();
    init.batch_size = Some(batch_size);
    init
}

async fn queue_event(logger: &support::Logger) {
    logger
        .async_log_success_event(support::event(support::standard_payload("one")))
        .await
        .expect("the event is accepted");
}

#[tokio::test]
async fn datadog_llm_obs_batch_queueing_behavior() {
    let environment = support::environment(&[]);
    let (logger, transport) = support::build(
        &threshold_init(2),
        &environment,
        vec![support::Reply::accepted(), support::Reply::accepted()],
    );
    queue_event(&logger).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.queue.pending().len(), 1);
    queue_event(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(transport.calls()[0].spans().len(), 2);
    assert_eq!(logger.queue.pending().len(), 0);

    flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn datadog_llm_obs_a_refused_threshold_batch_stays_queued() {
    let environment = support::environment(&[]);
    let (logger, transport) = support::build(
        &threshold_init(1),
        &environment,
        vec![support::Reply::status(500)],
    );
    queue_event(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn datadog_llm_obs_the_periodic_flush_drops_a_refused_batch() {
    let environment = support::environment(&[]);
    let (logger, transport) = support::build(
        &threshold_init(10),
        &environment,
        vec![support::Reply::status(500)],
    );
    queue_event(&logger).await;
    assert_eq!(logger.queue.pending().len(), 1);
    flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn datadog_llm_obs_the_periodic_timer_flushes_the_queue() {
    let environment = support::environment(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "3")]);
    let tracker = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &threshold_init(10),
        &environment,
        vec![support::Reply::accepted()],
        support::null_tracer(),
        Some(tracker),
    );
    queue_event(&logger).await;
    yield_now().await;
    assert_eq!(transport.call_count(), 0);
    advance(Duration::from_secs(3)).await;
    yield_now().await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(transport.calls()[0].spans().len(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn datadog_llm_obs_the_periodic_flush_stops_with_its_tracker() {
    let environment = support::environment(&[]);
    let tracker = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &threshold_init(10),
        &environment,
        vec![support::Reply::accepted(), support::Reply::accepted()],
        support::null_tracer(),
        Some(tracker.clone()),
    );
    queue_event(&logger).await;
    tracker.close();
    yield_now().await;
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    assert_eq!(transport.call_count(), 1);
    queue_event(&logger).await;
    advance(Duration::from_secs(5)).await;
    yield_now().await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 1);
}
