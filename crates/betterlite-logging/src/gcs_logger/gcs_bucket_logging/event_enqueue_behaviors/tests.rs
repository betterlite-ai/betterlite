#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

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
    for id in ["chatcmpl-1", "chatcmpl-2", "chatcmpl-3"] {
        enqueue(
            &harness.logger,
            &event(id, None, Json::Null, IndexMap::new()),
        )
        .await;
    }
    assert!(harness.requests().is_empty());
    assert_eq!(harness.queue_len(), 3);
    assert!(!logs.text().contains("still full"));
}

#[tokio::test]
async fn a_queue_still_full_after_the_flush_warns() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "0"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    let (logs, _guard) = capture_logs();
    for id in ["chatcmpl-1", "chatcmpl-2"] {
        enqueue(
            &harness.logger,
            &event(id, None, Json::Null, IndexMap::new()),
        )
        .await;
    }
    assert!(logs.text().contains("still full"));
}

#[tokio::test]
async fn a_flush_that_frees_room_does_not_warn() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    let (logs, _guard) = capture_logs();
    for id in ["chatcmpl-1", "chatcmpl-2"] {
        enqueue(
            &harness.logger,
            &event(id, None, Json::Null, IndexMap::new()),
        )
        .await;
    }
    assert!(!logs.text().contains("still full"));
}

#[tokio::test]
async fn a_zero_batch_size_flushes_nothing_and_keeps_the_item() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "0"),
        ],
        Some("bucket"),
        Answer::ok("{}"),
    );
    for id in ["chatcmpl-1", "chatcmpl-2"] {
        enqueue(
            &harness.logger,
            &event(id, None, Json::Null, IndexMap::new()),
        )
        .await;
    }
    assert!(harness.requests().is_empty());
    assert_eq!(harness.queue_len(), 2);
}

#[tokio::test]
async fn a_response_without_a_text_id_queues_an_empty_response_id() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    for response in [Json::Object(IndexMap::new()), Json::Null, Json::Bool(true)] {
        enqueue(&harness.logger, &event_with_response(response)).await;
    }
    let ids: Vec<String> = harness
        .queued()
        .iter()
        .map(|item| item.response_id.clone())
        .collect();
    assert_eq!(ids, vec![String::new(), String::new(), String::new()]);
}
