use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::Instant as TokioInstant;
use tokio::time::{sleep, timeout};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Level, Span};
use tracing_subscriber::fmt::MakeWriter;

use crate::worker::{
    Admission, Callback, DEFAULT_CONCURRENCY, DEFAULT_EVENT_TIMEOUT, DEFAULT_MAX_QUEUE_SIZE,
    DEFAULT_SHUTDOWN_DRAIN, DEFAULT_TIMEOUT_SUMMARY_WINDOW, FlushReport, LoggingWorker, Rejection,
    WorkerError, WorkerSettings,
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
    Panic,
}

#[derive(Debug, Default)]
struct SpanContexts(Mutex<BTreeMap<u64, &'static str>>);

impl SpanContexts {
    fn bind(&self, span: &Span, context: &'static str) {
        if let Some(id) = span.id() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.into_u64(), context);
        }
    }

    fn resolve(&self, span: &Span) -> String {
        let context = span.id().and_then(|id| {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id.into_u64())
                .copied()
        });
        context.unwrap_or("missing").to_owned()
    }
}

#[derive(Debug, Default)]
struct Record {
    entered: Mutex<Vec<&'static str>>,
    contexts: Mutex<Vec<(&'static str, String)>>,
}

impl Record {
    fn enter(&self, name: &'static str, spans: &SpanContexts) {
        self.entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        let context = spans.resolve(&Span::current());
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

    fn contexts(&self) -> Vec<(&'static str, String)> {
        self.contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct ScriptedCallback {
    name: &'static str,
    outcome: Outcome,
    record: Arc<Record>,
    spans: Arc<SpanContexts>,
    release: Option<Arc<Notify>>,
}

impl Callback for ScriptedCallback {
    type Error = ScriptedFailure;

    fn name(&self) -> &str {
        self.name
    }

    async fn call(self) -> Result<(), Self::Error> {
        self.record.enter(self.name, &self.spans);
        tracing::info!(callback = self.name, "the scripted callback ran");
        yield_now().await;
        if let Some(release) = self.release {
            release.notified().await;
        }
        match self.outcome {
            Outcome::Success | Outcome::Blocked => Ok(()),
            Outcome::Failure => Err(ScriptedFailure),
            Outcome::Hang => pending().await,
            Outcome::Panic => panic!("the scripted logging callback panicked"),
        }
    }
}

struct EagerPanicCallback {
    name: &'static str,
    panic_before_future: bool,
}

impl Callback for EagerPanicCallback {
    type Error = ScriptedFailure;

    fn name(&self) -> &str {
        self.name
    }

    fn call(self) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        assert!(
            !self.panic_before_future,
            "the scripted logging callback panicked before returning its future"
        );
        async { Ok(()) }
    }
}

fn callback(
    record: &Arc<Record>,
    name: &'static str,
    outcome: Outcome,
    spans: &Arc<SpanContexts>,
) -> ScriptedCallback {
    ScriptedCallback {
        name,
        outcome,
        record: Arc::clone(record),
        spans: Arc::clone(spans),
        release: None,
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

fn enqueue_in_span(
    worker: &LoggingWorker<ScriptedCallback>,
    record: &Arc<Record>,
    spans: &Arc<SpanContexts>,
    name: &'static str,
    context: &'static str,
) -> Admission {
    let span = tracing::info_span!("request_span", request = context);
    spans.bind(&span, context);
    let entered = span.enter();
    let admission = worker.enqueue(callback(record, name, Outcome::Success, spans));
    drop(entered);
    admission
}

fn respond_after_enqueue(
    worker: &LoggingWorker<ScriptedCallback>,
    callback: ScriptedCallback,
) -> (Admission, String) {
    let admission = worker.enqueue(callback);
    (admission, String::from("the successful model response"))
}

fn line_with<'text>(contents: &'text str, needle: &str) -> &'text str {
    contents
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or("")
}

async fn flush_within<C>(worker: &LoggingWorker<C>) -> FlushReport
where
    C: Callback,
{
    timeout(Duration::from_secs(2), worker.flush())
        .await
        .expect("the logging worker flush did not complete")
}

fn summary_lines(contents: &str) -> Vec<&str> {
    contents
        .lines()
        .filter(|line| line.contains("timed out and released their permits"))
        .collect()
}

fn capture_warnings() -> (Buffer, DefaultGuard) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

fn enqueue_hangs(
    worker: &LoggingWorker<ScriptedCallback>,
    record: &Arc<Record>,
    spans: &Arc<SpanContexts>,
    names: &[&'static str],
) {
    for name in names {
        let admission = worker.enqueue(callback(record, name, Outcome::Hang, spans));
        assert_eq!(admission, Admission::Accepted);
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
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    let _a = worker.enqueue(callback(&record, "a", Outcome::Success, &spans));
    let _b = worker.enqueue(callback(&record, "b", Outcome::Success, &spans));
    let _c = worker.enqueue(callback(&record, "c", Outcome::Success, &spans));
    let flush = flush_within(&worker).await;
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
    let (buffer, _guard) = capture_warnings();
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 1)).unwrap();
    let admission = worker.enqueue(callback(&record, "broken", Outcome::Failure, &spans));
    assert_eq!(admission, Admission::Accepted);
    let flush = flush_within(&worker).await;
    assert_eq!(flush.failed, 1);
    assert!(buffer.contents().contains("the logging callback failed"));
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.failed, 1);
    assert_eq!(shutdown.completed, 0);
}

#[tokio::test]
async fn a_panicking_callback_is_counted_under_its_span() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::INFO)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    let span = tracing::info_span!("request_span", request = "request-panic");
    spans.bind(&span, "request-panic");
    let entered = span.enter();
    let _panicking = worker.enqueue(callback(&record, "panicking", Outcome::Panic, &spans));
    drop(entered);
    let _next = worker.enqueue(callback(&record, "next", Outcome::Success, &spans));
    let flush = flush_within(&worker).await;
    assert_eq!(flush.failed, 1);
    assert_eq!(flush.completed, 1);
    let contents = buffer.contents();
    assert!(
        line_with(&contents, "a logging callback task failed")
            .contains(r#"request="request-panic""#)
    );
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.failed, 1);
    assert_eq!(shutdown.completed, 1);
}

#[tokio::test]
async fn a_callback_that_panics_before_its_future_is_counted() {
    let worker = LoggingWorker::start(settings(4, 1)).unwrap();
    let admission = worker.enqueue(EagerPanicCallback {
        name: "eager",
        panic_before_future: true,
    });
    assert_eq!(admission, Admission::Accepted);
    let flush = flush_within(&worker).await;
    assert_eq!(flush.failed, 1);
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.failed, 1);
}

#[tokio::test]
async fn each_event_keeps_the_context_captured_before_enqueue() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::INFO)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 3)).unwrap();
    let _first = enqueue_in_span(&worker, &record, &spans, "first", "request-a");
    let _second = enqueue_in_span(&worker, &record, &spans, "second", "request-b");
    let _third = enqueue_in_span(&worker, &record, &spans, "third", "request-c");
    let flush = flush_within(&worker).await;
    assert_eq!(flush.completed, 3);
    let mut contexts = record.contexts();
    contexts.sort_unstable();
    assert_eq!(
        contexts,
        vec![
            ("first", String::from("request-a")),
            ("second", String::from("request-b")),
            ("third", String::from("request-c")),
        ]
    );
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
}

#[tokio::test]
async fn a_callback_logs_under_the_span_captured_at_enqueue() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::INFO)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    let _first = enqueue_in_span(&worker, &record, &spans, "first", "request-a");
    let _second = enqueue_in_span(&worker, &record, &spans, "second", "request-b");
    let flush = flush_within(&worker).await;
    assert_eq!(flush.completed, 2);
    let contents = buffer.contents();
    assert!(line_with(&contents, r#"callback="first""#).contains(r#"request="request-a""#));
    assert!(line_with(&contents, r#"callback="second""#).contains(r#"request="request-b""#));
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
}

#[tokio::test(start_paused = true)]
async fn a_full_queue_rejects_only_the_new_event() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let release = Arc::new(Notify::new());
    let worker = LoggingWorker::start(settings(3, 1)).unwrap();
    let mut blocker = callback(&record, "blocker", Outcome::Blocked, &spans);
    blocker.release = Some(Arc::clone(&release));
    let _blocker = worker.enqueue(blocker);
    wait_for_names(&record, &["blocker"]).await;
    let _queued = worker.enqueue(callback(&record, "first", Outcome::Success, &spans));
    wait_for_dequeued(&worker, 2).await;
    let _a = worker.enqueue(callback(&record, "fill-a", Outcome::Success, &spans));
    let _b = worker.enqueue(callback(&record, "fill-b", Outcome::Success, &spans));
    let _c = worker.enqueue(callback(&record, "fill-c", Outcome::Success, &spans));
    let rejected = callback(&record, "rejected", Outcome::Success, &spans);
    let before = TokioInstant::now();
    let (admission, model_response) = respond_after_enqueue(&worker, rejected);
    assert_eq!(TokioInstant::now(), before);
    assert_eq!(admission, Admission::Rejected(Rejection::QueueFull));
    assert_eq!(worker.stats().rejected_full, 1);
    assert_eq!(model_response, "the successful model response");
    release.notify_one();
    let flush = flush_within(&worker).await;
    assert_eq!(flush.accepted, 5);
    assert_eq!(flush.completed, 5);
    let names = record.names();
    assert!(!names.contains(&"rejected"));
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.accepted, 5);
}

#[tokio::test]
async fn at_most_the_configured_callbacks_run_at_once() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let first_release = Arc::new(Notify::new());
    let second_release = Arc::new(Notify::new());
    let worker = LoggingWorker::start(settings(4, 1)).unwrap();
    let mut first = callback(&record, "first", Outcome::Blocked, &spans);
    first.release = Some(Arc::clone(&first_release));
    let _first = worker.enqueue(first);
    let mut second = callback(&record, "second", Outcome::Blocked, &spans);
    second.release = Some(Arc::clone(&second_release));
    let _second = worker.enqueue(second);
    wait_for_names(&record, &["first"]).await;
    wait_for_dequeued(&worker, 2).await;
    yield_now().await;
    yield_now().await;
    let stats = worker.stats();
    assert_eq!(stats.started, 1);
    assert_eq!(stats.dequeued, 2);
    assert_eq!(record.names(), ["first"]);
    first_release.notify_one();
    wait_for_names(&record, &["first", "second"]).await;
    second_release.notify_one();
    let flush = flush_within(&worker).await;
    assert_eq!(flush.completed, 2);
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_callback_releases_its_permit() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let settings = timed_settings(
        1,
        Duration::from_millis(20),
        Duration::from_secs(1),
        Duration::from_millis(20),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _hang = worker.enqueue(callback(&record, "hang", Outcome::Hang, &spans));
    let _next = worker.enqueue(callback(&record, "next", Outcome::Success, &spans));
    let flush = flush_within(&worker).await;
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
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(8, 2)).unwrap();
    let _a = worker.enqueue(callback(&record, "a", Outcome::Success, &spans));
    let _b = worker.enqueue(callback(&record, "b", Outcome::Success, &spans));
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    assert_eq!(shutdown.accepted, 2);
    assert_eq!(shutdown.completed, 2);
    assert_eq!(shutdown.cancelled, 0);
    assert_eq!(shutdown.dropped, 0);
    let settled = shutdown
        .completed
        .saturating_add(shutdown.failed)
        .saturating_add(shutdown.timed_out)
        .saturating_add(shutdown.cancelled)
        .saturating_add(shutdown.dropped);
    assert_eq!(shutdown.accepted, settled);
}

#[tokio::test(start_paused = true)]
async fn stop_accounts_for_events_left_at_the_deadline() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let settings = timed_settings(
        1,
        Duration::from_secs(5),
        Duration::from_millis(30),
        Duration::from_millis(20),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _hang = worker.enqueue(callback(&record, "hang", Outcome::Hang, &spans));
    let _waiting = worker.enqueue(callback(&record, "waiting", Outcome::Success, &spans));
    wait_for_names(&record, &["hang"]).await;
    let shutdown = worker.stop().await;
    assert!(!shutdown.drained);
    assert_eq!(shutdown.accepted, 2);
    assert_eq!(shutdown.completed, 0);
    assert_eq!(shutdown.cancelled, 1);
    assert_eq!(shutdown.dropped, 1);
    let settled = shutdown
        .completed
        .saturating_add(shutdown.failed)
        .saturating_add(shutdown.timed_out)
        .saturating_add(shutdown.cancelled)
        .saturating_add(shutdown.dropped);
    assert_eq!(shutdown.accepted, settled);
    let flush = flush_within(&worker).await;
    assert_eq!(flush.accepted, 2);
    assert_eq!(flush.completed, 0);
}

#[tokio::test(start_paused = true)]
async fn stop_cancels_running_callbacks_when_the_deadline_expires() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let first_release = Arc::new(Notify::new());
    let second_release = Arc::new(Notify::new());
    let settings = timed_settings(
        2,
        Duration::from_secs(5),
        Duration::from_millis(30),
        Duration::from_millis(20),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let mut first = callback(&record, "first", Outcome::Blocked, &spans);
    first.release = Some(first_release);
    let _first = worker.enqueue(first);
    let mut second = callback(&record, "second", Outcome::Blocked, &spans);
    second.release = Some(second_release);
    let _second = worker.enqueue(second);
    wait_for_names(&record, &["first", "second"]).await;
    let shutdown = worker.stop().await;
    assert!(!shutdown.drained);
    assert_eq!(shutdown.accepted, 2);
    assert_eq!(shutdown.completed, 0);
    assert_eq!(shutdown.cancelled, 2);
    assert_eq!(shutdown.dropped, 0);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_burst_is_summarized_once() {
    let (buffer, _guard) = capture_warnings();
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let settings = timed_settings(
        1,
        Duration::from_millis(10),
        Duration::from_secs(1),
        Duration::from_millis(50),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    enqueue_hangs(
        &worker,
        &record,
        &spans,
        &["first", "second", "third", "fourth", "fifth"],
    );
    let flush = flush_within(&worker).await;
    assert_eq!(flush.timed_out, 5);
    sleep(Duration::from_millis(20)).await;
    let contents = buffer.contents();
    let summaries = summary_lines(&contents);
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].contains("callbacks=5"));
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.timed_out, 5);
}

#[tokio::test(start_paused = true)]
async fn timeout_bursts_are_summarized_once_per_window() {
    let (buffer, _guard) = capture_warnings();
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let settings = timed_settings(
        1,
        Duration::from_millis(10),
        Duration::from_secs(1),
        Duration::from_millis(25),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    enqueue_hangs(
        &worker,
        &record,
        &spans,
        &[
            "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth",
        ],
    );
    sleep(Duration::from_millis(40)).await;
    let first_contents = buffer.contents();
    let first_window = summary_lines(&first_contents);
    assert_eq!(first_window.len(), 1);
    assert!(first_window[0].contains("callbacks=3"));
    assert!(worker.stats().started < 8);
    let flush = flush_within(&worker).await;
    assert_eq!(flush.timed_out, 8);
    sleep(Duration::from_millis(30)).await;
    let contents = buffer.contents();
    let summaries = summary_lines(&contents);
    assert_eq!(summaries.len(), 3);
    assert!(summaries[0].contains("callbacks=3"));
    assert!(summaries[1].contains("callbacks=3"));
    assert!(summaries[2].contains("callbacks=2"));
    let shutdown = worker.stop().await;
    assert_eq!(shutdown.timed_out, 8);
}

#[tokio::test(start_paused = true)]
async fn stop_emits_an_open_timeout_window_at_the_deadline() {
    let (buffer, _guard) = capture_warnings();
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let settings = timed_settings(
        1,
        Duration::from_millis(20),
        Duration::from_millis(10),
        Duration::from_millis(50),
    );
    let worker = LoggingWorker::start(settings).unwrap();
    let _timed = worker.enqueue(callback(&record, "timed", Outcome::Hang, &spans));
    let _running = worker.enqueue(callback(&record, "running", Outcome::Hang, &spans));
    sleep(Duration::from_millis(15)).await;
    let shutdown = worker.stop().await;
    assert!(!shutdown.drained);
    assert_eq!(shutdown.timed_out, 1);
    assert_eq!(shutdown.cancelled, 1);
    let contents = buffer.contents();
    let summaries = summary_lines(&contents);
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].contains("callbacks=1"));
}

#[tokio::test]
async fn admission_reports_a_closed_worker() {
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let worker = LoggingWorker::start(settings(4, 1)).unwrap();
    let shutdown = worker.stop().await;
    assert!(shutdown.drained);
    let repeated = worker.stop().await;
    assert!(repeated.drained);
    assert_eq!(repeated.accepted, 0);
    assert_eq!(
        worker.enqueue(callback(&record, "late", Outcome::Success, &spans)),
        Admission::Rejected(Rejection::Closed)
    );
    assert_eq!(worker.stats().rejected_closed, 1);
}

async fn wait_for_names(record: &Arc<Record>, wanted: &[&'static str]) {
    timeout(Duration::from_secs(2), async {
        while !wanted.iter().all(|name| record.names().contains(name)) {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the scripted callbacks did not start");
}

async fn wait_for_dequeued(worker: &LoggingWorker<ScriptedCallback>, depth: usize) {
    timeout(Duration::from_secs(2), async {
        while worker.stats().dequeued < depth {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the worker did not dequeue the events");
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
