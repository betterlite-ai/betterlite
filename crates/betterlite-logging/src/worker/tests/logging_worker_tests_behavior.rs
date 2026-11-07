use std::future::pending;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::worker::{
    Admission, Callback, DEFAULT_CONCURRENCY, DEFAULT_EVENT_TIMEOUT, DEFAULT_MAX_QUEUE_SIZE,
    DEFAULT_SHUTDOWN_DRAIN, DEFAULT_TIMEOUT_SUMMARY_WINDOW, LoggingWorker, Rejection, WorkerError,
    WorkerSettings,
};

#[derive(Debug, thiserror::Error)]
#[error("the scripted logging callback failed")]
struct ScriptedFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Success,
    Failure,
    Hang,
    Blocked,
}

#[derive(Debug, Default)]
struct Record {
    entered: Mutex<Vec<&'static str>>,
    contexts: Mutex<Vec<(&'static str, &'static str)>>,
}

impl Record {
    fn enter(&self, name: &'static str, context: &'static str) {
        self.entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        self.contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name, context));
    }

    fn names(&self) -> Vec<&'static str> {
        self.entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn contexts(&self) -> Vec<(&'static str, &'static str)> {
        self.contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct ScriptedCallback {
    name: &'static str,
    outcome: Outcome,
    context: &'static str,
    record: Arc<Record>,
    release: Option<Arc<Notify>>,
}

impl Callback for ScriptedCallback {
    type Error = ScriptedFailure;

    fn name(&self) -> &str {
        self.name
    }

    async fn call(self) -> Result<(), Self::Error> {
        self.record.enter(self.name, self.context);
        yield_now().await;
        if let Some(release) = self.release {
            release.notified().await;
        }
        match self.outcome {
            Outcome::Success | Outcome::Blocked => Ok(()),
            Outcome::Failure => Err(ScriptedFailure),
            Outcome::Hang => pending().await,
        }
    }
}

fn callback(record: &Arc<Record>, name: &'static str, outcome: Outcome) -> ScriptedCallback {
    ScriptedCallback {
        name,
        outcome,
        context: "request-default",
        record: Arc::clone(record),
        release: None,
    }
}

fn contextual(record: &Arc<Record>, name: &'static str, context: &'static str) -> ScriptedCallback {
    ScriptedCallback {
        name,
        outcome: Outcome::Success,
        context,
        record: Arc::clone(record),
        release: None,
    }
}

fn blocked(record: &Arc<Record>, name: &'static str, release: Arc<Notify>) -> ScriptedCallback {
    ScriptedCallback {
        name,
        outcome: Outcome::Blocked,
        context: "request-blocked",
        record: Arc::clone(record),
        release: Some(release),
    }
}

fn settings(max_queue_size: usize, concurrency: usize) -> WorkerSettings {
    WorkerSettings {
        max_queue_size,
        concurrency,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(1),
        timeout_summary_window: Duration::from_millis(20),
    }
}

fn timed_settings(
    concurrency: usize,
    event_timeout: Duration,
    shutdown_drain: Duration,
    window: Duration,
) -> WorkerSettings {
    WorkerSettings {
        max_queue_size: 8,
        concurrency,
        event_timeout,
        shutdown_drain,
        timeout_summary_window: window,
    }
}

#[test]
fn defaults_match_the_litellm_worker() {
    let settings = WorkerSettings::default();
    assert_eq!(settings.max_queue_size, DEFAULT_MAX_QUEUE_SIZE);
    assert_eq!(settings.concurrency, DEFAULT_CONCURRENCY);
    assert_eq!(settings.event_timeout, DEFAULT_EVENT_TIMEOUT);
    assert_eq!(settings.shutdown_drain, DEFAULT_SHUTDOWN_DRAIN);
    assert_eq!(
        settings.timeout_summary_window,
        DEFAULT_TIMEOUT_SUMMARY_WINDOW
    );
    assert_eq!(DEFAULT_MAX_QUEUE_SIZE, 50_000);
    assert_eq!(DEFAULT_CONCURRENCY, 100);
    assert_eq!(DEFAULT_EVENT_TIMEOUT, Duration::from_secs(20));
    assert_eq!(DEFAULT_SHUTDOWN_DRAIN, Duration::from_secs(30));
}

#[rstest]
#[case::zero_queue_size(
    WorkerSettings {
        max_queue_size: 0,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(1),
        timeout_summary_window: Duration::from_secs(5),
    },
    "queue size"
)]
#[case::zero_concurrency(
    WorkerSettings {
        max_queue_size: 8,
        concurrency: 0,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(1),
        timeout_summary_window: Duration::from_secs(5),
    },
    "concurrency"
)]
fn invalid_limits_are_rejected(#[case] settings: WorkerSettings, #[case] expected: &str) {
    match LoggingWorker::<ScriptedCallback>::start(settings) {
        Ok(_worker) => panic!("the worker accepted invalid limits"),
        Err(error) => {
            assert!(matches!(
                &error,
                WorkerError::ZeroQueueSize | WorkerError::ZeroConcurrency
            ));
            assert!(error.to_string().contains(expected));
        }
    }
}

#[tokio::test]
async fn successful_callbacks_run_and_flush_waits_for_them() {
    let record = Arc::new(Record::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    assert_eq!(
        worker.enqueue(callback(&record, "a", Outcome::Success)),
        Admission::Accepted
    );
    assert_eq!(
        worker.enqueue(callback(&record, "b", Outcome::Success)),
        Admission::Accepted
    );
    assert_eq!(
        worker.enqueue(callback(&record, "c", Outcome::Success)),
        Admission::Accepted
    );
    let flush = worker.flush().await;
    assert_eq!(flush.accepted, 3);
    assert_eq!(flush.completed, 3);
    assert_eq!(flush.failed, 0);
    let stats = worker.stats();
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.completed, 3);
    let mut names = record.names();
    names.sort_unstable();
    assert_eq!(names, ["a", "b", "c"]);
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.accepted, 3);
    assert_eq!(shutdown.completed, 3);
}

#[tokio::test]
async fn a_failed_callback_is_isolated_from_the_model_response() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let record = Arc::new(Record::default());
    let model_response = String::from("the successful model response");
    let worker = LoggingWorker::start(settings(8, 1)).unwrap();
    let _accepted = worker.enqueue(callback(&record, "broken", Outcome::Failure));
    let flush = worker.flush().await;
    assert_eq!(flush.failed, 1);
    assert_eq!(model_response, "the successful model response");
    assert!(buffer.contents().contains("the logging callback failed"));
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.failed, 1);
    assert_eq!(shutdown.completed, 0);
}

#[tokio::test]
async fn each_event_keeps_the_context_captured_before_enqueue() {
    let record = Arc::new(Record::default());
    let worker = LoggingWorker::start(settings(8, 3)).unwrap();
    let _first = worker.enqueue(contextual(&record, "first", "request-a"));
    let _second = worker.enqueue(contextual(&record, "second", "request-b"));
    let _third = worker.enqueue(contextual(&record, "third", "request-c"));
    let flush = worker.flush().await;
    assert_eq!(flush.completed, 3);
    let mut contexts = record.contexts();
    contexts.sort_unstable();
    assert_eq!(
        contexts,
        [
            ("first", "request-a"),
            ("second", "request-b"),
            ("third", "request-c")
        ]
    );
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
}

#[tokio::test]
async fn a_full_queue_rejects_only_the_new_event() {
    let record = Arc::new(Record::default());
    let release = Arc::new(Notify::new());
    let worker = LoggingWorker::start(settings(3, 1)).unwrap();
    let _blocker = worker.enqueue(blocked(&record, "blocker", Arc::clone(&release)));
    wait_for_names(&record, &["blocker"]).await;
    let _queued = worker.enqueue(callback(&record, "first", Outcome::Success));
    wait_for_dequeued(&worker, 2).await;
    let _a = worker.enqueue(callback(&record, "fill-a", Outcome::Success));
    let _b = worker.enqueue(callback(&record, "fill-b", Outcome::Success));
    let _c = worker.enqueue(callback(&record, "fill-c", Outcome::Success));
    let rejected = callback(&record, "rejected", Outcome::Success);
    assert_eq!(
        worker.enqueue(rejected),
        Admission::Rejected(Rejection::QueueFull)
    );
    assert_eq!(worker.stats().rejected_full, 1);
    release.notify_one();
    let flush = worker.flush().await;
    assert_eq!(flush.accepted, 5);
    assert_eq!(flush.completed, 5);
    let names = record.names();
    assert!(!names.contains(&"rejected"));
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.accepted, 5);
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_callback_releases_its_permit() {
    let record = Arc::new(Record::default());
    let settings = timed_settings(
        1,
        Duration::from_millis(20),
        Duration::from_secs(1),
        Duration::from_millis(20),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _hang = worker.enqueue(callback(&record, "hang", Outcome::Hang));
    let _next = worker.enqueue(callback(&record, "next", Outcome::Success));
    let flush = worker.flush().await;
    assert_eq!(flush.timed_out, 1);
    assert_eq!(flush.completed, 1);
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.timed_out, 1);
    assert_eq!(shutdown.completed, 1);
    assert_eq!(shutdown.cancelled, 0);
}

#[tokio::test]
async fn stop_drains_accepted_events_before_the_deadline() {
    let record = Arc::new(Record::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    let _a = worker.enqueue(callback(&record, "a", Outcome::Success));
    let _b = worker.enqueue(callback(&record, "b", Outcome::Success));
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.accepted, 2);
    assert_eq!(shutdown.completed, 2);
    assert_eq!(shutdown.cancelled, 0);
    assert_eq!(shutdown.dropped, 0);
}

#[tokio::test(start_paused = true)]
async fn stop_accounts_for_events_left_at_the_deadline() {
    let record = Arc::new(Record::default());
    let settings = timed_settings(
        1,
        Duration::from_secs(5),
        Duration::from_millis(30),
        Duration::from_millis(20),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _hang = worker.enqueue(callback(&record, "hang", Outcome::Hang));
    let _waiting = worker.enqueue(callback(&record, "waiting", Outcome::Success));
    wait_for_names(&record, &["hang"]).await;
    let shutdown = worker.stop().await;
    assert!(!shutdown.drained);
    assert_eq!(shutdown.accepted, 2);
    assert_eq!(shutdown.completed, 0);
    assert_eq!(shutdown.cancelled, 1);
    assert_eq!(shutdown.dropped, 1);
    let flush = worker.flush().await;
    assert_eq!(flush.accepted, 2);
    assert_eq!(flush.completed, 0);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_burst_is_summarized_once() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let record = Arc::new(Record::default());
    let settings = timed_settings(
        2,
        Duration::from_millis(10),
        Duration::from_secs(1),
        Duration::from_millis(50),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _first = worker.enqueue(callback(&record, "first", Outcome::Hang));
    let _second = worker.enqueue(callback(&record, "second", Outcome::Hang));
    let flush = worker.flush().await;
    assert_eq!(flush.timed_out, 2);
    sleep(Duration::from_millis(60)).await;
    let contents = buffer.contents();
    assert_eq!(
        contents
            .matches("timed out and released their permits")
            .count(),
        1
    );
    assert!(contents.contains("callbacks=2"));
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.timed_out, 2);
}

#[tokio::test]
async fn admission_reports_a_closed_worker() {
    let record = Arc::new(Record::default());
    let worker = LoggingWorker::start(settings(4, 1)).unwrap();
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    let repeated = worker.stop().await;
    assert!(repeated.drained);
    assert_eq!(repeated.accepted, 0);
    assert_eq!(
        worker.enqueue(callback(&record, "late", Outcome::Success)),
        Admission::Rejected(Rejection::Closed)
    );
    assert_eq!(worker.stats().rejected_closed, 1);
}

async fn wait_for_names(record: &Arc<Record>, wanted: &[&'static str]) {
    timeout(Duration::from_secs(2), async {
        while !wanted.iter().all(|name| record.names().contains(name)) {
            yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_dequeued(worker: &LoggingWorker<ScriptedCallback>, depth: usize) {
    timeout(Duration::from_secs(2), async {
        while worker.stats().dequeued < depth {
            yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}
