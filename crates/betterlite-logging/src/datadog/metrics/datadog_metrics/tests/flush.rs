use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;
use tokio_util::task::TaskTracker;

use crate::datadog::metrics::flush::flush_queue;

use super::support::{self, FixedClock, MetricsInit, NOW_DURATION, Outcome};

const API_KEY: &str = "secret-key";

fn environment() -> support::Environment {
    support::environment(&[("DD_API_KEY", API_KEY)])
}

#[tokio::test]
async fn flush_sends_nothing_for_an_empty_queue() {
    let (logger, transport) = support::build(&environment(), MetricsInit::default(), Vec::new());
    let report = flush_queue(&logger).await;
    assert_eq!(report.batch, 0);
    assert_eq!(report.delivered, 0);
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn flush_clears_the_queue_after_a_successful_send() {
    let (logger, transport) = support::build(
        &environment(),
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let report = flush_queue(&logger).await;
    assert_eq!(report.batch, 2);
    assert!(logger.queue.pending().is_empty());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn flush_sends_at_the_batch_size() {
    let (logger, transport) = support::build(
        &environment(),
        MetricsInit {
            batch_size: Some(2),
        },
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    assert_eq!(transport.call_count(), 1);
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn flush_waits_below_the_batch_size() {
    let (logger, transport) = support::build(
        &environment(),
        MetricsInit {
            batch_size: Some(3),
        },
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.queue.pending().len(), 2);
    let _report = flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn flush_runs_every_five_seconds() {
    let tasks = TaskTracker::new();
    let (logger, transport) = support::build_with(
        &environment(),
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
        Arc::new(FixedClock(NOW_DURATION)),
        Some(tasks),
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    yield_now().await;
    advance(Duration::from_secs(4)).await;
    assert_eq!(transport.call_count(), 0);
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(transport.call_count(), 1);
    assert!(logger.queue.pending().is_empty());
}
