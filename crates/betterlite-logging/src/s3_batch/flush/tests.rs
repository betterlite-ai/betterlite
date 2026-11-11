#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::s3_batch::Element;
use crate::s3_batch::flush::{
    FlushContext, FlushError, FlushSettings, UploadOperation, send_batch,
};
use crate::s3_batch::ndjson_batch_file::{BatchIdSource, UuidBatchIds};
use crate::s3_batch::queue::BatchQueue;
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::UploadOutcome;
use betterlite_models::decisions::Json;

#[derive(Debug, Default)]
struct ScriptedUpload {
    outcomes: Mutex<VecDeque<UploadOutcome>>,
    uploaded: Mutex<Vec<String>>,
}

impl ScriptedUpload {
    fn with(outcomes: &[UploadOutcome]) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            uploaded: Mutex::new(Vec::new()),
        }
    }

    fn uploaded(&self) -> Vec<String> {
        self.uploaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn take_uploaded(&self) -> Vec<String> {
        let mut uploaded = self.uploaded.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *uploaded)
    }
}

impl UploadOperation for ScriptedUpload {
    fn upload<'operation>(
        &'operation self,
        element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>> {
        self.uploaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(element.s3_object_key.clone());
        let outcome = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(UploadOutcome::Delivered);
        Box::pin(async move { outcome })
    }
}

// the first upload appends an event, standing in for an event logged while the flush awaits S3
struct ArrivingUpload<'queue> {
    queue: &'queue BatchQueue,
    arrival: Element,
    outcomes: Mutex<VecDeque<UploadOutcome>>,
    arrived: Mutex<bool>,
}

impl<'queue> ArrivingUpload<'queue> {
    fn new(
        queue: &'queue BatchQueue,
        arrival: Element,
        outcomes: &[UploadOutcome],
    ) -> ArrivingUpload<'queue> {
        Self {
            queue,
            arrival,
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            arrived: Mutex::new(false),
        }
    }
}

impl UploadOperation for ArrivingUpload<'_> {
    fn upload<'operation>(
        &'operation self,
        _element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>> {
        let mut arrived = self.arrived.lock().unwrap_or_else(PoisonError::into_inner);
        if !*arrived {
            *arrived = true;
            self.queue.enqueue(self.arrival.clone());
        }
        let outcome = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(UploadOutcome::Delivered);
        Box::pin(async move { outcome })
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Debug)]
struct FixedIds(Mutex<usize>);

impl BatchIdSource for FixedIds {
    fn next_batch_id(&self) -> String {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = format!("id{}", *next);
        *next = next.saturating_add(1);
        id
    }
}

fn element(key: &str, id: &str) -> Element {
    element_since(key, id, None)
}

fn element_since(key: &str, id: &str, retrying_since: Option<f64>) -> Element {
    let payload: Json =
        serde_json::from_str(&format!(r#"{{"id":"{id}"}}"#)).expect("the payload is JSON");
    Element {
        payload,
        s3_object_key: key.to_owned(),
        s3_object_download_filename: format!("{id}.json"),
        body: None,
        content_type: ContentType::Json,
        retrying_since,
    }
}

fn settings(max_retry_age: Option<Duration>, max_queue_size: usize) -> FlushSettings {
    FlushSettings {
        max_retry_age,
        max_queue_size,
        batch_file_upload: false,
        cold_storage_logger: None,
        max_concurrent_uploads: 16,
    }
}

fn clock() -> FixedClock {
    FixedClock(SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_323_045))
}

async fn run(
    queue: &BatchQueue,
    settings: &FlushSettings,
    now: f64,
    upload: &dyn UploadOperation,
) -> Result<usize, FlushError> {
    let ids = UuidBatchIds;
    let clock = clock();
    let context = FlushContext {
        queue,
        settings,
        batch_ids: &ids,
        clock: &clock,
        now,
    };
    send_batch(&context, upload).await
}

fn keys(queue: &BatchQueue) -> Vec<String> {
    queue
        .snapshot()
        .entries
        .into_iter()
        .map(|element| element.s3_object_key)
        .collect()
}

#[tokio::test]
async fn fresh_work_ordered() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let first = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    let error = run(&queue, &settings, 0.0, &first)
        .await
        .expect_err("one upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(ref failure) if failure.failed == 1 && failure.total == 2
    ));
    assert_eq!(queue.snapshot().requeued_count, 1);
    queue.enqueue(element("c", "c"));
    first.take_uploaded();

    let second = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Delivered]);
    let batch_len = run(&queue, &settings, 1.0, &second)
        .await
        .expect("every upload is delivered");
    assert_eq!(batch_len, 2);
    assert_eq!(second.uploaded(), vec!["c".to_owned(), "b".to_owned()]);
}

#[tokio::test]
async fn requeued_preserved_on_total_failure() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element_since("stale", "stale", Some(0.0)));
    queue.enqueue(element("terminal", "terminal"));
    queue.enqueue(element("transient", "transient"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Retry,
        UploadOutcome::Dropped,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 10_000.0, &upload)
        .await
        .expect_err("nothing was delivered");
    assert!(matches!(
        error,
        FlushError::Upload(ref failure) if failure.failed == 3 && failure.total == 3
    ));
    let restored = queue.snapshot();
    assert_eq!(
        restored
            .entries
            .iter()
            .map(|element| element.s3_object_key.as_str())
            .collect::<Vec<_>>(),
        vec!["stale", "terminal", "transient"]
    );
    assert_eq!(restored.requeued_count, 3);
    assert_eq!(restored.entries[0].retrying_since, Some(0.0));
    assert_eq!(restored.entries[2].retrying_since, None);
}

#[tokio::test]
async fn partial_success_drops_stale_and_terminal() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("stale", "stale", Some(1_000.0)));
    queue.enqueue(element("terminal", "terminal"));
    queue.enqueue(element("fresh", "fresh"));
    let settings = settings(Some(Duration::from_secs(100)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Delivered,
        UploadOutcome::Retry,
        UploadOutcome::Dropped,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 2_000.0, &upload)
        .await
        .expect_err("one upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(ref failure) if failure.failed == 3 && failure.total == 4
    ));
    let restored = queue.snapshot();
    assert_eq!(keys(&queue), vec!["fresh"]);
    assert_eq!(restored.requeued_count, 1);
    assert_eq!(restored.entries[0].retrying_since, Some(2_000.0));
}

#[tokio::test]
async fn overflow_trims_oldest() {
    let queue = BatchQueue::with_capacity(2);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    queue.enqueue(element("c", "c"));
    let settings = settings(None, 2);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Retry,
        UploadOutcome::Retry,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 0.0, &upload)
        .await
        .expect_err("every upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(ref failure) if failure.failed == 3 && failure.total == 3
    ));
    assert_eq!(queue.snapshot().requeued_count, 2);
    assert_eq!(keys(&queue), vec!["b", "c"]);
}

#[tokio::test]
async fn preserve_retrying_since_on_requeue() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("remembered", "remembered", Some(100.0)));
    queue.enqueue(element("first", "first"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Delivered,
        UploadOutcome::Retry,
        UploadOutcome::Retry,
    ]);
    run(&queue, &settings, 500.0, &upload)
        .await
        .expect_err("two uploads survive");
    let restored = queue.snapshot();
    assert_eq!(
        restored
            .entries
            .iter()
            .map(|element| element.retrying_since)
            .collect::<Vec<_>>(),
        vec![Some(100.0), Some(500.0)]
    );
}

#[tokio::test]
async fn raises_only_on_survivors() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let delivered = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Delivered]);
    assert_eq!(run(&queue, &settings, 0.0, &delivered).await, Ok(2));
    queue.finish_success(2);

    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let retried = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    assert!(run(&queue, &settings, 0.0, &retried).await.is_err());

    queue.finish_success(queue.snapshot().entries.len());
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let dropped = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Dropped]);
    assert_eq!(run(&queue, &settings, 0.0, &dropped).await, Ok(2));
    assert_eq!(keys(&queue), vec!["a", "b"]);
}

#[tokio::test]
async fn preserve_events_added_during_flush() {
    let queue = BatchQueue::with_capacity(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);

    let success = ArrivingUpload::new(
        &queue,
        element("arrival", "arrival"),
        &[UploadOutcome::Delivered, UploadOutcome::Delivered],
    );
    let batch_len = run(&queue, &settings, 0.0, &success)
        .await
        .expect("both uploads are delivered");
    queue.finish_success(batch_len);
    assert_eq!(keys(&queue), vec!["arrival"]);

    queue.finish_success(queue.snapshot().entries.len());
    queue.enqueue(element("c", "c"));
    queue.enqueue(element("d", "d"));
    let failure = ArrivingUpload::new(
        &queue,
        element("late", "late"),
        &[UploadOutcome::Retry, UploadOutcome::Retry],
    );
    run(&queue, &settings, 0.0, &failure)
        .await
        .expect_err("both uploads survive");
    assert_eq!(keys(&queue), vec!["c", "d", "late"]);
    assert_eq!(queue.snapshot().requeued_count, 2);
}

#[test]
fn upload_error_reports_its_counts() {
    let error = crate::s3_batch::error::S3BatchUploadError {
        failed: 2,
        total: 5,
    };
    assert_eq!(
        error.to_string(),
        "2 of 5 S3 uploads failed; transient failures kept in queue for the next flush"
    );
}
