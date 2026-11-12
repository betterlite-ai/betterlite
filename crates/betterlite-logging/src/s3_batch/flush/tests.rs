#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::mem::take;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::s3_batch::Element;
use crate::s3_batch::S3BatchUploadError;
use crate::s3_batch::flush::{
    FlushContext, FlushError, FlushSettings, MonotonicClock, SystemMonotonic, UploadOperation,
    send_batch,
};
use crate::s3_batch::queue::{BatchQueue, QueueState};
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::UploadOutcome;
use betterlite_models::decisions::Json;
use tokio::time::sleep;

#[derive(Debug, Default)]
struct ScriptedUpload<'queue> {
    outcomes: Mutex<VecDeque<UploadOutcome>>,
    uploaded: Mutex<Vec<String>>,
    arrival: Option<(&'queue BatchQueue, Element)>,
    arrived: Mutex<bool>,
}

impl<'queue> ScriptedUpload<'queue> {
    fn with(outcomes: &[UploadOutcome]) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            uploaded: Mutex::new(Vec::new()),
            arrival: None,
            arrived: Mutex::new(false),
        }
    }

    fn arriving(queue: &'queue BatchQueue, arrival: Element, outcomes: &[UploadOutcome]) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            uploaded: Mutex::new(Vec::new()),
            arrival: Some((queue, arrival)),
            arrived: Mutex::new(false),
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
        take(&mut *uploaded)
    }

    fn record(&self, element: &Element) {
        self.uploaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(element.s3_object_key.clone());
    }

    fn next(&self) -> UploadOutcome {
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(UploadOutcome::Delivered)
    }
}

impl UploadOperation for ScriptedUpload<'_> {
    fn upload<'operation>(
        &'operation self,
        element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>> {
        self.record(element);
        if let Some((queue, arrival)) = &self.arrival {
            let mut arrived = self.arrived.lock().unwrap_or_else(PoisonError::into_inner);
            if !*arrived {
                *arrived = true;
                queue.enqueue(arrival.clone());
            }
            drop(arrived);
        }
        let outcome = self.next();
        Box::pin(async move { outcome })
    }
}

#[derive(Debug)]
struct ManualMonotonic(Mutex<f64>);

impl MonotonicClock for ManualMonotonic {
    fn seconds(&self) -> f64 {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct AdvancingUpload<'clock> {
    clock: &'clock ManualMonotonic,
    after: f64,
    inner: ScriptedUpload<'static>,
}

impl UploadOperation for AdvancingUpload<'_> {
    fn upload<'operation>(
        &'operation self,
        element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>> {
        *self.clock.0.lock().unwrap_or_else(PoisonError::into_inner) = self.after;
        self.inner.upload(element)
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

fn queue(capacity: usize) -> BatchQueue {
    BatchQueue {
        state: Mutex::new(QueueState::default()),
        max_queue_size: capacity,
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
    let time = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_323_045))
        .expect("the clock fits");
    FixedClock(time)
}

async fn run(
    queue: &BatchQueue,
    settings: &FlushSettings,
    now: f64,
    upload: &dyn UploadOperation,
) -> Result<usize, FlushError> {
    let clock = clock();
    let monotonic = ManualMonotonic(Mutex::new(now));
    let context = FlushContext {
        queue,
        settings,
        clock: &clock,
        monotonic: &monotonic,
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
    let queue = queue(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let first = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    let error = run(&queue, &settings, 0.0_f64, &first)
        .await
        .expect_err("one upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(failure) if failure.failed == 1 && failure.total == 2
    ));
    assert_eq!(queue.snapshot().requeued_count, 1);
    queue.enqueue(element("c", "c"));
    first.take_uploaded();

    let second = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Delivered]);
    let batch_len = run(&queue, &settings, 1.0_f64, &second)
        .await
        .expect("every upload is delivered");
    assert_eq!(batch_len, 2);
    assert_eq!(second.uploaded(), vec!["c".to_owned(), "b".to_owned()]);
}

#[tokio::test]
async fn requeued_preserved_on_total_failure() {
    let queue = queue(10);
    queue.enqueue(element_since("stale", "stale", Some(0.0_f64)));
    queue.enqueue(element("terminal", "terminal"));
    queue.enqueue(element("transient", "transient"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Retry,
        UploadOutcome::Dropped,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 10_000.0_f64, &upload)
        .await
        .expect_err("nothing was delivered");
    assert!(matches!(
        error,
        FlushError::Upload(failure) if failure.failed == 3 && failure.total == 3
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
    assert_eq!(restored.entries[0].retrying_since, Some(0.0_f64));
    assert_eq!(restored.entries[2].retrying_since, None);
}

#[tokio::test]
async fn partial_success_drops_stale_and_terminal() {
    let queue = queue(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("stale", "stale", Some(1_000.0_f64)));
    queue.enqueue(element("terminal", "terminal"));
    queue.enqueue(element("fresh", "fresh"));
    let settings = settings(Some(Duration::from_secs(100)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Delivered,
        UploadOutcome::Retry,
        UploadOutcome::Dropped,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 2_000.0_f64, &upload)
        .await
        .expect_err("one upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(failure) if failure.failed == 3 && failure.total == 4
    ));
    let restored = queue.snapshot();
    assert_eq!(keys(&queue), vec!["fresh"]);
    assert_eq!(restored.requeued_count, 1);
    assert_eq!(restored.entries[0].retrying_since, Some(2_000.0_f64));
}

#[tokio::test]
async fn overflow_trims_oldest() {
    let queue = queue(2);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    queue.enqueue(element("c", "c"));
    let settings = settings(None, 2);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Retry,
        UploadOutcome::Retry,
        UploadOutcome::Retry,
    ]);
    let error = run(&queue, &settings, 0.0_f64, &upload)
        .await
        .expect_err("every upload survives");
    assert!(matches!(
        error,
        FlushError::Upload(failure) if failure.failed == 3 && failure.total == 3
    ));
    assert_eq!(queue.snapshot().requeued_count, 2);
    assert_eq!(keys(&queue), vec!["b", "c"]);
}

#[tokio::test]
async fn preserve_retrying_since_on_requeue() {
    let queue = queue(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("remembered", "remembered", Some(100.0_f64)));
    queue.enqueue(element("first", "first"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let upload = ScriptedUpload::with(&[
        UploadOutcome::Delivered,
        UploadOutcome::Retry,
        UploadOutcome::Retry,
    ]);
    run(&queue, &settings, 500.0_f64, &upload)
        .await
        .expect_err("two uploads survive");
    let restored = queue.snapshot();
    assert_eq!(
        restored
            .entries
            .iter()
            .map(|element| element.retrying_since)
            .collect::<Vec<_>>(),
        vec![Some(100.0_f64), Some(500.0_f64)]
    );
}

#[tokio::test]
async fn raises_only_on_survivors() {
    let queue = queue(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);
    let delivered = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Delivered]);
    assert_eq!(
        run(&queue, &settings, 0.0_f64, &delivered)
            .await
            .expect("every upload is delivered"),
        2
    );
    queue.finish_success(2);

    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let retried = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    run(&queue, &settings, 0.0_f64, &retried)
        .await
        .expect_err("one upload survives");

    queue.finish_success(queue.snapshot().entries.len());
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let dropped = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Dropped]);
    assert_eq!(
        run(&queue, &settings, 0.0_f64, &dropped)
            .await
            .expect("nothing survives the flush"),
        2
    );
    assert_eq!(keys(&queue), vec!["a", "b"]);
}

#[tokio::test]
async fn preserve_events_added_during_flush() {
    let queue = queue(10);
    queue.enqueue(element("a", "a"));
    queue.enqueue(element("b", "b"));
    let settings = settings(Some(Duration::from_secs(3_600)), 10);

    let success = ScriptedUpload::arriving(
        &queue,
        element("arrival", "arrival"),
        &[UploadOutcome::Delivered, UploadOutcome::Delivered],
    );
    let batch_len = run(&queue, &settings, 0.0_f64, &success)
        .await
        .expect("both uploads are delivered");
    queue.finish_success(batch_len);
    assert_eq!(keys(&queue), vec!["arrival"]);

    queue.finish_success(queue.snapshot().entries.len());
    queue.enqueue(element("c", "c"));
    queue.enqueue(element("d", "d"));
    let failure = ScriptedUpload::arriving(
        &queue,
        element("late", "late"),
        &[UploadOutcome::Retry, UploadOutcome::Retry],
    );
    run(&queue, &settings, 0.0_f64, &failure)
        .await
        .expect_err("both uploads survive");
    assert_eq!(keys(&queue), vec!["c", "d", "late"]);
    assert_eq!(queue.snapshot().requeued_count, 2);
}

#[tokio::test]
async fn retry_age_limit_is_exclusive() {
    let queue = queue(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("edge", "edge", Some(100.0_f64)));
    let settings = settings(Some(Duration::from_secs(100)), 10);
    let upload = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    run(&queue, &settings, 200.0_f64, &upload)
        .await
        .expect_err("the element at the age limit survives");
    assert_eq!(keys(&queue), vec!["edge"]);
    assert_eq!(queue.snapshot().entries[0].retrying_since, Some(100.0_f64));

    let aged = self::queue(10);
    aged.enqueue(element("delivered", "delivered"));
    aged.enqueue(element_since("over", "over", Some(99.0_f64)));
    let late = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    run(&aged, &settings, 200.0_f64, &late)
        .await
        .expect("the element past the age limit is dropped, so nothing survives");
}

#[tokio::test]
async fn retrying_since_stays_unset_without_a_retry_age() {
    let queue = queue(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element("fresh", "fresh"));
    let settings = settings(None, 10);
    let upload = ScriptedUpload::with(&[UploadOutcome::Delivered, UploadOutcome::Retry]);
    run(&queue, &settings, 500.0_f64, &upload)
        .await
        .expect_err("one upload survives");
    assert_eq!(queue.snapshot().entries[0].retrying_since, None);
}

#[tokio::test]
async fn retry_age_is_measured_after_the_uploads() {
    let queue = queue(10);
    queue.enqueue(element("delivered", "delivered"));
    queue.enqueue(element_since("old", "old", Some(0.0_f64)));
    queue.enqueue(element("fresh", "fresh"));
    let settings = settings(Some(Duration::from_secs(10)), 10);
    let clock = clock();
    let monotonic = ManualMonotonic(Mutex::new(5.0_f64));
    let context = FlushContext {
        queue: &queue,
        settings: &settings,
        clock: &clock,
        monotonic: &monotonic,
    };
    let upload = AdvancingUpload {
        clock: &monotonic,
        after: 20.0_f64,
        inner: ScriptedUpload::with(&[
            UploadOutcome::Delivered,
            UploadOutcome::Retry,
            UploadOutcome::Retry,
        ]),
    };
    send_batch(&context, &upload)
        .await
        .expect_err("the fresh upload survives");
    let restored = queue.snapshot().entries;
    assert_eq!(keys(&queue), vec!["fresh"]);
    assert_eq!(restored[0].retrying_since, Some(20.0_f64));
}

#[tokio::test]
async fn monotonic_seconds_start_near_zero_and_advance() {
    let clock = SystemMonotonic::default();
    let first = clock.seconds();
    assert!((0.0_f64..1.0_f64).contains(&first));
    sleep(Duration::from_millis(5)).await;
    assert!(clock.seconds() > first);
}

#[test]
fn upload_error_reports_its_counts() {
    let error = S3BatchUploadError {
        failed: 2,
        total: 5,
    };
    assert_eq!(
        error.to_string(),
        "2 of 5 S3 uploads failed; transient failures kept in queue for the next flush"
    );
}
