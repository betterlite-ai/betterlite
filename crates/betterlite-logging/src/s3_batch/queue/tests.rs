#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::s3_batch::Element;
use crate::s3_batch::queue::BatchQueue;
use crate::s3_transport::batch_element::ContentType;

fn element(key: &str) -> Element {
    let payload: Json = serde_json::from_str(r#"{"id":"request"}"#).expect("the payload is JSON");
    Element {
        payload,
        s3_object_key: key.to_owned(),
        s3_object_download_filename: "download.json".to_owned(),
        body: None,
        content_type: ContentType::Json,
        retrying_since: None,
    }
}

fn keys(queue: &BatchQueue) -> Vec<String> {
    queue
        .snapshot()
        .entries
        .into_iter()
        .map(|element| element.s3_object_key)
        .collect()
}

#[test]
fn finish_success_keeps_events_added_meanwhile() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("a"));
    queue.enqueue(element("b"));
    queue.enqueue(element("arrival"));
    queue.finish_success(2);
    assert_eq!(keys(&queue), vec!["arrival"]);
    assert_eq!(queue.snapshot().requeued_count, 0);
}

#[test]
fn restore_puts_requeued_ahead_of_arrivals() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("a"));
    queue.enqueue(element("arrival"));
    let restored = queue.restore_after_failure(1, vec![element("b")]);
    assert_eq!(restored.dropped_oldest, 0);
    assert_eq!(keys(&queue), vec!["b", "arrival"]);
    assert_eq!(queue.snapshot().requeued_count, 1);
}

#[test]
fn restore_trims_the_oldest_and_adjusts_the_requeued_count() {
    let queue = BatchQueue::with_capacity(2);
    queue.enqueue(element("a"));
    queue.enqueue(element("arrival"));
    let restored = queue.restore_after_failure(1, vec![element("b"), element("c")]);
    assert_eq!(restored.dropped_oldest, 1);
    assert_eq!(keys(&queue), vec!["c", "arrival"]);
    assert_eq!(queue.snapshot().requeued_count, 1);
}

#[test]
fn trim_overflow_drops_the_oldest_entries() {
    let queue = BatchQueue::with_capacity(2);
    queue.enqueue(element("a"));
    queue.enqueue(element("b"));
    queue.enqueue(element("c"));
    let trimmed = queue.trim_overflow();
    assert_eq!(trimmed.dropped_oldest, 1);
    assert_eq!(keys(&queue), vec!["b", "c"]);
}
