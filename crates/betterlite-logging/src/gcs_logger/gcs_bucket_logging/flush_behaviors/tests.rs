#![cfg(test)]

use std::time::Duration;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::super::event_enqueue_behaviors::enqueue;
use super::super::tests::support::{Answer, event, now, open, open_scheduled};
use super::flush_queue;

#[tokio::test]
async fn flushing_sends_the_queued_items() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    flush_queue(&harness.logger).await;
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(harness.queue_len(), 0);
}

#[tokio::test]
async fn flushing_an_empty_queue_sends_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    flush_queue(&harness.logger).await;
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn flushing_updates_the_last_flush_time() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    assert_eq!(harness.last_flush_time(), now());
    let later = DateTime::parse_from_rfc3339("2024-03-10T00:00:05Z")
        .expect("the instant is valid")
        .with_timezone(&Utc);
    harness.clock.set(later);
    flush_queue(&harness.logger).await;
    assert_eq!(harness.last_flush_time(), later);
}

#[tokio::test(start_paused = true)]
async fn callback_logger_flushes_on_the_interval() {
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let harness = open_scheduled(
        &[("GCS_FLUSH_INTERVAL", "1")],
        Some("bucket"),
        Answer::ok("{}"),
        &tracker,
        &shutdown,
    );
    enqueue(
        &harness.logger,
        &event("chatcmpl-1", None, Json::Null, IndexMap::new()),
    )
    .await;
    tokio::task::yield_now().await;
    assert!(harness.requests().is_empty());
    advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(harness.queue_len(), 0);
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test(start_paused = true)]
async fn callback_logger_stops_on_shutdown() {
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let harness = open_scheduled(&[], Some("bucket"), Answer::ok("{}"), &tracker, &shutdown);
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    assert!(harness.requests().is_empty());
}
