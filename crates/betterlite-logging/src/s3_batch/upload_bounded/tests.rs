#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::iter::repeat_with;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use betterlite_config::loader::EnvironmentLookup;
use futures_util::future::join_all;
use tokio::task::yield_now;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::s3_batch::Element;
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use crate::s3_batch::upload_bounded::{AcquireError, UploadLimiter};
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response, ThrottleSink};
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;
use betterlite_models::decisions::Json;

#[derive(Debug, Default)]
struct ScriptedClient {
    replies: Mutex<VecDeque<(u16, String)>>,
    observed_uploads: Mutex<Vec<bool>>,
    watcher: Mutex<Option<Weak<S3BatchLogger>>>,
}

impl ScriptedClient {
    fn with_replies(replies: &[(u16, &str)]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|(status, body)| (*status, (*body).to_owned()))
                    .collect(),
            ),
            observed_uploads: Mutex::new(Vec::new()),
            watcher: Mutex::new(None),
        })
    }

    fn watch(&self, logger: &Arc<S3BatchLogger>) {
        *self.watcher.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(logger));
    }

    fn observed_uploads(&self) -> Vec<bool> {
        self.observed_uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl S3HttpClient for ScriptedClient {
    fn send(
        &self,
        _request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        let active = self
            .watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|logger| logger.in_flight_flushes.load(Ordering::SeqCst) > 0);
        self.observed_uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(active);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or((200, String::new()));
        Box::pin(async move {
            Ok(S3Response {
                status: reply.0,
                body: reply.1.into_bytes(),
            })
        })
    }
}

#[derive(Debug, Default)]
struct RecordingSink {
    samples: Mutex<Vec<bool>>,
}

impl RecordingSink {
    fn samples(&self) -> Vec<bool> {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ThrottleSink for RecordingSink {
    fn record(&self, throttled: bool) {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(throttled);
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

fn source(extra: &str) -> Source<()> {
    serde_json::from_str(&format!(
        r#"{{"s3_bucket_name":"logs","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000"{extra}}}"#
    ))
    .expect("the S3 source loads")
}

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

fn logger(
    source: &Source<()>,
    client: Arc<ScriptedClient>,
    observer: Option<Arc<RecordingSink>>,
) -> Arc<S3BatchLogger> {
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), source, None, environment.as_ref());
    let throttle_observer = observer.map(|sink| {
        let shared: Arc<dyn ThrottleSink> = sink;
        shared
    });
    S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client,
            clock: Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment,
            throttle_observer,
            cold_storage_logger: None,
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    )
    .expect("the logger builds")
}

fn deadline(millis: u64) -> Instant {
    Instant::now()
        .checked_add(Duration::from_millis(millis))
        .expect("the deadline fits")
}

#[tokio::test]
async fn bounded_by_limit() {
    let limiter = Arc::new(UploadLimiter::new(2, None).expect("the bounds hold"));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let uploads = repeat_with(|| {
        let limiter_handle = Arc::clone(&limiter);
        let active_handle = Arc::clone(&active);
        let peak_handle = Arc::clone(&peak);
        async move {
            let permit = limiter_handle
                .acquire(deadline(5_000))
                .await
                .expect("a permit arrives");
            let running = active_handle
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            peak_handle.fetch_max(running, Ordering::SeqCst);
            yield_now().await;
            yield_now().await;
            active_handle.fetch_sub(1, Ordering::SeqCst);
            drop(permit);
        }
    })
    .take(6);
    join_all(uploads).await;
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn limiter_returns_semaphore() {
    let limiter = UploadLimiter::new(1, None).expect("the bounds hold");
    assert_eq!(limiter.snapshot().limit, 1);
    let permit = limiter
        .acquire(deadline(5_000))
        .await
        .expect("the permit is free");
    assert_eq!(limiter.snapshot().limit, 0);
    let blocked = limiter.acquire(deadline(1)).await;
    assert_eq!(blocked.err(), Some(AcquireError::DeadlineExceeded));
    drop(permit);
    assert_eq!(limiter.snapshot().limit, 1);
}

#[tokio::test(start_paused = true)]
async fn permit_wait_is_bounded_by_its_deadline() {
    let limiter = UploadLimiter::new(1, Some(3)).expect("the bounds hold");
    let permit = limiter
        .acquire(deadline(5_000))
        .await
        .expect("the permit is free");
    let blocked = limiter.acquire(deadline(5)).await;
    assert_eq!(blocked.err(), Some(AcquireError::DeadlineExceeded));
    drop(permit);
    drop(limiter);
}

#[tokio::test]
async fn context_var_set() {
    let client = ScriptedClient::with_replies(&[(200, "")]);
    let logger = logger(&source(""), Arc::clone(&client), None);
    client.watch(&logger);
    assert_eq!(logger.in_flight_flushes.load(Ordering::SeqCst), 0);
    logger.queue.enqueue(element("a.json"));
    logger.flush_queue().await.expect("the upload is delivered");
    assert_eq!(client.observed_uploads(), vec![true]);
    assert_eq!(logger.in_flight_flushes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn adaptive_samples_record() {
    let observer = Arc::new(RecordingSink::default());
    let client = ScriptedClient::with_replies(&[(429, ""), (200, "")]);
    let logger = logger(
        &source(
            r#","s3_adaptive_concurrency":true,"s3_max_concurrent_uploads":1,"s3_max_adaptive_concurrency":3"#,
        ),
        Arc::clone(&client),
        Some(Arc::clone(&observer)),
    );
    logger.queue.enqueue(element("a.json"));
    let error = logger.flush_queue().await;
    error.expect_err("a 429 keeps the upload for the next flush");
    logger.flush_queue().await.expect("the retry is delivered");
    assert_eq!(observer.samples(), vec![true, false]);
    assert_eq!(logger.uploads.snapshot().limit, 2);
    assert_eq!(
        logger.queue.snapshot().entries.len(),
        0,
        "the delivered upload left the queue"
    );
    drop(logger);
}

#[tokio::test]
async fn adaptive_limiter_grows_and_halves() {
    let limiter = UploadLimiter::new(1, Some(3)).expect("the bounds hold");
    limiter.record(false);
    assert_eq!(limiter.snapshot().limit, 2);
    limiter.record(false);
    limiter.record(false);
    assert_eq!(limiter.snapshot().limit, 3);
    limiter.record(true);
    assert_eq!(limiter.snapshot().limit, 1);
}

#[tokio::test(start_paused = true)]
async fn halving_with_permits_held_shrinks_the_pool_when_they_return() {
    let limiter = UploadLimiter::new(1, Some(4)).expect("the bounds hold");
    limiter.record(false);
    limiter.record(false);
    limiter.record(false);
    assert_eq!(limiter.snapshot().limit, 3);
    let first_held = limiter
        .acquire(deadline(5))
        .await
        .expect("a permit is free");
    let second_held = limiter
        .acquire(deadline(5))
        .await
        .expect("a permit is free");
    let third_held = limiter
        .acquire(deadline(5))
        .await
        .expect("a permit is free");
    limiter.record(true);
    assert_eq!(limiter.snapshot().limit, 1);
    drop((first_held, second_held, third_held));
    let first = limiter
        .acquire(deadline(5))
        .await
        .expect("one permit remains");
    let second = limiter.acquire(deadline(5)).await;
    assert_eq!(second.err(), Some(AcquireError::DeadlineExceeded));
    drop(first);
}

#[test]
fn fixed_limiter_has_a_semaphore_limit() {
    let limiter = UploadLimiter::new(4, None).expect("the bounds hold");
    assert_eq!(limiter.snapshot().limit, 4);
}

#[test]
fn adaptive_bounds_are_validated() {
    UploadLimiter::new(0, Some(3)).expect_err("a zero window has no permits");
    UploadLimiter::new(4, Some(3)).expect_err("the window exceeds the ceiling");
    UploadLimiter::new(1, Some(1)).expect("the bounds hold");
}
