#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::event_enqueue_behaviors::enqueue;
use super::super::tests::support::{Answer, dynamic, event, open, upload_name};
use super::async_send_batch;

#[tokio::test]
async fn batched_mode_uploads_one_grouped_object() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    async_send_batch(&harness.logger).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    let name = upload_name(&requests[0]).expect("the batch carries a name");
    assert!(name.contains("/batch-"));
    assert!(name.ends_with(".ndjson"));
    assert_eq!(harness.queue_len(), 0);
}

#[tokio::test]
async fn individual_mode_uploads_each_item() {
    let harness = open(
        &[("GCS_USE_BATCHED_LOGGING", "false")],
        Some("bucket"),
        Answer::ok("{}"),
    );
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    async_send_batch(&harness.logger).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 2);
    let names: Vec<String> = requests.iter().filter_map(upload_name).collect();
    assert_eq!(
        names,
        vec![
            "2024-03-09/chatcmpl-1".to_owned(),
            "2024-03-09/chatcmpl-2".to_owned()
        ]
    );
    assert_eq!(harness.queue_len(), 0);
}

#[tokio::test]
async fn only_the_batch_size_is_sent() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "0"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    async_send_batch(&harness.logger).await;
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(harness.queue_len(), 1);
    assert_eq!(harness.queued()[0].payload.id, "chatcmpl-2");
}

#[tokio::test]
async fn an_empty_drain_sends_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    async_send_batch(&harness.logger).await;
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn each_config_group_becomes_its_own_upload() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    enqueue(
        &harness.logger,
        &event(
            "chatcmpl-1",
            None,
            dynamic(&[("gcs_bucket_name", "a")]),
            IndexMap::new(),
        ),
    )
    .await;
    enqueue(
        &harness.logger,
        &event(
            "chatcmpl-2",
            None,
            dynamic(&[("gcs_bucket_name", "b")]),
            IndexMap::new(),
        ),
    )
    .await;
    async_send_batch(&harness.logger).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url().contains("/b/a/o"));
    assert!(requests[1].url().contains("/b/b/o"));
}

#[tokio::test]
async fn an_invalid_dynamic_config_drops_the_batch() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    enqueue(
        &harness.logger,
        &event("chatcmpl-1", None, Json::Bool(true), IndexMap::new()),
    )
    .await;
    async_send_batch(&harness.logger).await;
    assert!(harness.requests().is_empty());
    assert_eq!(harness.queue_len(), 0);
}
