#![cfg(test)]

use super::super::tests::support::{Answer, open};
use super::drain_queue_batch;

#[tokio::test]
async fn the_drain_stops_at_the_batch_size() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "0"),
            ("GCS_BATCH_SIZE", "2"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    harness
        .enqueue_all(&["chatcmpl-1", "chatcmpl-2", "chatcmpl-3"])
        .await;
    let first = drain_queue_batch(&harness.logger);
    assert_eq!(
        first
            .iter()
            .map(|item| item.payload.id.clone())
            .collect::<Vec<_>>(),
        vec!["chatcmpl-1".to_owned(), "chatcmpl-2".to_owned()]
    );
    assert_eq!(harness.queue_len(), 1);
    let second = drain_queue_batch(&harness.logger);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].payload.id, "chatcmpl-3");
    assert!(drain_queue_batch(&harness.logger).is_empty());
}

#[tokio::test]
async fn an_empty_queue_drains_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    assert!(drain_queue_batch(&harness.logger).is_empty());
    assert_eq!(harness.queue_len(), 0);
}

#[tokio::test]
async fn the_drain_takes_items_in_queue_order() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "0"),
            ("GCS_BATCH_SIZE", "10"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    harness
        .enqueue_all(&["chatcmpl-1", "chatcmpl-2", "chatcmpl-3"])
        .await;
    let drained = drain_queue_batch(&harness.logger);
    assert_eq!(
        drained
            .iter()
            .map(|item| item.payload.id.clone())
            .collect::<Vec<_>>(),
        vec![
            "chatcmpl-1".to_owned(),
            "chatcmpl-2".to_owned(),
            "chatcmpl-3".to_owned()
        ]
    );
    assert_eq!(harness.queue_len(), 0);
}
