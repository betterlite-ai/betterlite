use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    Buffer, FakeIngest, as_row_ingest, event, event_with, expected_ids, logger, params, payload,
    queued_ids, settle, subscribe,
};

fn deep_json(depth: usize) -> Json {
    (0..depth).fold(Json::Text("leaf".to_owned()), |inner, _| {
        Json::Object(IndexMap::from([("next".to_owned(), inner)]))
    })
}

#[tokio::test]
async fn zerobus_enqueue_appends_a_row_per_event() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    assert_eq!(
        queued_ids(&logger),
        expected_ids(&["chatcmpl-a", "chatcmpl-b"])
    );
}

#[tokio::test]
async fn zerobus_enqueue_starts_the_batch_flush_at_the_batch_size() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(2, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    assert_eq!(queued_ids(&logger), expected_ids(&["chatcmpl-a"]));
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert_eq!(ingest.ids(), expected_ids(&["chatcmpl-a", "chatcmpl-b"]));
    assert_eq!(ingest.inserted(), 1);
    assert!(queued_ids(&logger).is_empty());
}

#[tokio::test]
async fn zerobus_enqueue_holds_rows_below_the_batch_size() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(3, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 0);
    assert_eq!(
        queued_ids(&logger),
        expected_ids(&["chatcmpl-a", "chatcmpl-b"])
    );
}

#[tokio::test]
async fn zerobus_enqueue_swallows_a_payload_that_cannot_be_read() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, false), client);
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    let mut deep = payload("chatcmpl-deep");
    deep.model_parameters
        .insert("deep".to_owned(), deep_json(200));
    logger
        .async_log_success_event(event_with(
            deep,
            Json::Object(IndexMap::new()),
            IndexMap::new(),
        ))
        .await
        .expect("the event is accepted even when the row cannot be built");
    drop(guard);
    assert!(queued_ids(&logger).is_empty());
    assert!(
        buffer.contents().contains("failed to queue an event"),
        "{}",
        buffer.contents()
    );
}
