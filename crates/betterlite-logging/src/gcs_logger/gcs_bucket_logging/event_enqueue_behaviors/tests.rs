#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::tests::support::{
    Answer, capture_logs, dynamic, event, event_with_response, metadata, open,
};
use super::enqueue;

#[tokio::test]
async fn a_success_event_queues_its_payload_and_response_id() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    enqueue(
        &harness.logger,
        &event("chatcmpl-1", None, Json::Null, IndexMap::new()),
    )
    .await;
    let queued = harness.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload.id, "chatcmpl-1");
    assert_eq!(queued[0].response_id, "chatcmpl-1");
    assert!(queued[0].payload.error_str.is_none());
}

#[tokio::test]
async fn a_failure_event_keeps_the_error_string() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    enqueue(
        &harness.logger,
        &event(
            "chatcmpl-2",
            Some("the upstream call failed"),
            Json::Null,
            IndexMap::new(),
        ),
    )
    .await;
    let queued = harness.queued();
    assert_eq!(
        queued[0].payload.error_str.as_deref(),
        Some("the upstream call failed")
    );
}

#[tokio::test]
async fn the_item_carries_the_dynamic_params_and_metadata() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let dynamic_params = dynamic(&[("gcs_bucket_name", "dynamic-bucket")]);
    let litellm_metadata = metadata("gcs_log_id", Json::Text("my-run".to_owned()));
    enqueue(
        &harness.logger,
        &event(
            "chatcmpl-1",
            None,
            dynamic_params.clone(),
            litellm_metadata.clone(),
        ),
    )
    .await;
    let queued = harness.queued();
    assert_eq!(queued[0].dynamic_params.as_ref(), &dynamic_params);
    assert_eq!(queued[0].metadata.as_ref(), &litellm_metadata);
}

#[tokio::test]
async fn a_full_queue_flushes_before_enqueueing() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    enqueue(
        &harness.logger,
        &event("chatcmpl-1", None, Json::Null, IndexMap::new()),
    )
    .await;
    assert_eq!(harness.queue_len(), 1);
    assert!(harness.requests().is_empty());
    enqueue(
        &harness.logger,
        &event("chatcmpl-2", None, Json::Null, IndexMap::new()),
    )
    .await;
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(harness.queue_len(), 1);
    assert_eq!(harness.queued()[0].payload.id, "chatcmpl-2");
}

#[tokio::test]
async fn an_unbounded_queue_never_flushes_on_enqueue() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "0"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    let (logs, _guard) = capture_logs();
    harness
        .enqueue_all(&["chatcmpl-1", "chatcmpl-2", "chatcmpl-3"])
        .await;
    assert!(harness.requests().is_empty());
    assert_eq!(harness.queue_len(), 3);
    assert!(!logs.text().contains("still full"));
}

#[tokio::test]
async fn a_queue_still_full_after_the_flush_drops_the_new_item() {
    let mut harness = open(
        &[("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1")],
        Some("bucket"),
        Answer::ok("{}"),
    );
    Arc::get_mut(&mut harness.logger)
        .expect("the harness holds the only handle")
        .settings
        .batch_size = 0;
    let (logs, _guard) = capture_logs();
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    assert!(harness.requests().is_empty());
    assert_eq!(harness.queue_len(), 1);
    assert_eq!(harness.queued()[0].payload.id, "chatcmpl-1");
    assert!(logs.text().contains("still full"));
}

#[tokio::test]
async fn a_flush_that_frees_room_keeps_the_new_item_without_an_error() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    let (logs, _guard) = capture_logs();
    harness.enqueue_all(&["chatcmpl-1", "chatcmpl-2"]).await;
    assert_eq!(harness.queue_len(), 1);
    assert_eq!(harness.queued()[0].payload.id, "chatcmpl-2");
    assert!(!logs.text().contains("still full"));
}

#[rstest]
#[case::empty_object(Json::Object(IndexMap::new()))]
#[case::null(Json::Null)]
#[case::boolean(Json::Bool(true))]
#[tokio::test]
async fn a_response_without_a_text_id_queues_an_empty_response_id(#[case] response: Json) {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    enqueue(&harness.logger, &event_with_response(response)).await;
    assert_eq!(harness.queued()[0].response_id, "");
}
