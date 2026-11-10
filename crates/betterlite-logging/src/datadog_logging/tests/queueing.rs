use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;

use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_logging::batch;
use crate::datadog_logging::logger::DatadogLogger;
use crate::datadog_logging::tests::support;

async fn settle() {
    for _ in 0_u8..8_u8 {
        yield_now().await;
    }
}

async fn queue_two(logger: &DatadogLogger) {
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    logger
        .async_log_failure_event(support::event(support::standard_payload("two", true)))
        .await
        .expect("the async event is accepted");
}

#[tokio::test]
async fn datadog_batch_queueing_behavior() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    queue_two(&logger).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.queue.pending().len(), 2);

    batch::flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(transport.calls()[0].entries().len(), 2);
    assert_eq!(logger.queue.pending().len(), 0);

    batch::flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn datadog_batch_queueing_updates_last_flush_time_on_success() {
    let clock = Arc::new(support::SequenceClock::new(&[
        Duration::from_secs(10),
        Duration::from_secs(20),
    ]));
    let (logger, _transport) = support::build_with(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
        clock,
        None,
    );
    assert_eq!(support::last_flush_time(&logger), Duration::from_secs(10));
    queue_two(&logger).await;
    batch::flush_queue(&logger).await;
    assert_eq!(support::last_flush_time(&logger), Duration::from_secs(20));
}

#[tokio::test]
async fn datadog_batch_queueing_keeps_last_flush_time_on_failure() {
    let clock = Arc::new(support::SequenceClock::new(&[
        Duration::from_secs(10),
        Duration::from_secs(20),
    ]));
    let (logger, _transport) = support::build_with(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::status(500)],
        clock,
        None,
    );
    queue_two(&logger).await;
    batch::flush_queue(&logger).await;
    assert_eq!(support::last_flush_time(&logger), Duration::from_secs(10));
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn periodic_flush_sends_queued_events_after_the_interval() {
    let tasks = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted(), support::Reply::accepted()],
        Arc::new(support::FixedClock(Duration::from_secs(1))),
        Some(tasks.clone()),
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    assert_eq!(transport.call_count(), 0);
    settle().await;
    advance(Duration::from_secs(5)).await;
    settle().await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    batch::stop(&logger).await;
    tasks.close();
    tasks.wait().await;
    advance(Duration::from_secs(15)).await;
    settle().await;
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn closing_the_tracker_stops_the_periodic_flush() {
    let tasks = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
        Arc::new(support::FixedClock(Duration::from_secs(1))),
        Some(tasks.clone()),
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    settle().await;
    tasks.close();
    advance(Duration::from_secs(5)).await;
    settle().await;
    assert_eq!(transport.call_count(), 0);
    batch::stop(&logger).await;
    tasks.wait().await;
}

#[tokio::test]
async fn stop_flushes_the_pending_queue() {
    let tasks = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
        Arc::new(support::FixedClock(Duration::from_secs(1))),
        Some(tasks.clone()),
    );
    queue_two(&logger).await;
    assert_eq!(transport.call_count(), 0);
    batch::stop(&logger).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    tasks.close();
    tasks.wait().await;
}

#[test]
fn a_logger_without_a_tracker_starts_no_timer() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        Vec::new(),
    );
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.queue.pending().len(), 0);
}
