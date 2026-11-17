use std::io::{self, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use futures_util::stream::{self, StreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::sync::mpsc;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use betterlite_config::sections::service_callback::Settings as ServiceCallbackSettings;
use betterlite_db::database_diagnostics::level::LogLevel;

use crate::logging_service_events::clock::SystemClock;
use crate::logging_service_events::pipeline::{
    CALL_ID_HEADER, PipelineConfig, PipelineError, ServicePipeline,
};
use crate::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use crate::logging_service_events::stream::STREAM_FAILED_ERROR;
use crate::logging_service_events::types::{
    CANCELLED_ERROR, ServiceEvent, ServiceKind, ServiceLoggerPayload, ServiceMetric, ServiceType,
    TerminalOutcome,
};
use crate::logging_service_events::wiring::LoggingWiring;
use crate::logging_service_events::worker_link::Delivery;
use crate::logging_streams::assembler::ChatStreamAssembler;
use crate::logging_success::ports::SuccessClock;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::{CallbackList, CallbackPhase};
use crate::registry::sink::CallbackSink;
use crate::registry::{CallbackRegistry, Registration};
use crate::worker::{Callback, WorkerError, WorkerSettings};

const NOW: &str = "2026-01-01T00:00:00+00:00";
const SECRET: &str = "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// Two clock reads per call: the pre-call event takes the first and the terminal event the second, so
// the recorded duration is exactly one step and both timestamps are predictable.
struct SteppingClock {
    base: DateTime<FixedOffset>,
    step: i64,
    reads: AtomicU32,
}

impl SteppingClock {
    fn new() -> Arc<Self> {
        Self::with_step(1)
    }

    fn with_step(step: i64) -> Arc<Self> {
        let base = DateTime::parse_from_rfc3339(NOW).unwrap();
        Arc::new(Self {
            base,
            step,
            reads: AtomicU32::new(0),
        })
    }

    fn base_time(&self) -> DateTime<FixedOffset> {
        self.base
    }
}

impl SuccessClock for SteppingClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let step = i64::from(self.reads.fetch_add(1, Ordering::SeqCst)) * self.step;
        self.base + TimeDelta::seconds(step)
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<ServiceEvent>>,
}

impl RecordingSink {
    fn recorded(&self) -> Vec<ServiceEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ServiceEventSink for RecordingSink {
    fn emit(&self, event: &ServiceEvent) -> Result<(), ServiceEventError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
        Ok(())
    }
}

struct FailingSink;

impl ServiceEventSink for FailingSink {
    fn emit(&self, _event: &ServiceEvent) -> Result<(), ServiceEventError> {
        Err(ServiceEventError::Rejected {
            message: "the sink is unavailable".to_owned(),
        })
    }
}

struct Harness {
    pipeline: Arc<ServicePipeline>,
    sink: Arc<RecordingSink>,
    clock: Arc<SteppingClock>,
}

fn harness_with(
    sink: Arc<RecordingSink>,
    registry: CallbackRegistry,
    settings: WorkerSettings,
) -> Harness {
    let clock = SteppingClock::new();
    let pipeline = Arc::new(
        ServicePipeline::start(PipelineConfig {
            settings,
            clock: Arc::clone(&clock),
            sink: Arc::clone(&sink),
            registry: Arc::new(registry),
        })
        .unwrap(),
    );
    Harness {
        pipeline,
        sink,
        clock,
    }
}

fn harness() -> Harness {
    harness_with(
        Arc::new(RecordingSink::default()),
        CallbackRegistry::new(64),
        WorkerSettings::default(),
    )
}

fn headers_with(id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CALL_ID_HEADER, HeaderValue::from_str(id).unwrap());
    headers
}

fn header_value(headers: &HeaderMap) -> String {
    headers
        .get(CALL_ID_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn begin_generates_a_call_id_and_writes_the_header() {
    let harness = harness();
    let mut headers = HeaderMap::new();
    drop(harness.pipeline.begin(ServiceKind::Chat, &mut headers));
    harness.pipeline.worker.flush().await;
    let header = header_value(&headers);
    assert!(uuid::Uuid::parse_str(&header).is_ok());
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].call_id, header);
    assert_eq!(events[1].call_id, header);
}

#[tokio::test]
async fn begin_keeps_a_client_call_id() {
    let harness = harness();
    let mut headers = headers_with("client-call-id");
    drop(harness.pipeline.begin(ServiceKind::Chat, &mut headers));
    harness.pipeline.worker.flush().await;
    assert_eq!(header_value(&headers), "client-call-id");
    assert_eq!(harness.sink.recorded()[0].call_id, "client-call-id");
}

#[tokio::test]
async fn begin_replaces_an_oversized_call_id() {
    let harness = harness();
    let oversized = "x".repeat(257);
    let mut headers = headers_with(&oversized);
    drop(harness.pipeline.begin(ServiceKind::Chat, &mut headers));
    harness.pipeline.worker.flush().await;
    let header = header_value(&headers);
    assert!(uuid::Uuid::parse_str(&header).is_ok());
    assert_eq!(harness.sink.recorded()[0].call_id, header);
}

#[tokio::test]
async fn begin_redacts_a_secret_call_id() {
    let harness = harness();
    let mut headers = headers_with(SECRET);
    drop(harness.pipeline.begin(ServiceKind::Chat, &mut headers));
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events[0].call_id, "REDACTED");
    assert_eq!(events[1].call_id, "REDACTED");
    assert_eq!(header_value(&headers), SECRET);
}

#[tokio::test]
async fn pre_call_event_carries_the_call_identity() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    drop(harness.pipeline.begin(ServiceKind::Embedding, &mut headers));
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    let pre_call = &events[0];
    assert_eq!(pre_call.call_id, "call-1");
    assert_eq!(pre_call.payload.service, ServiceType::ProxyPreCall);
    assert_eq!(pre_call.payload.call_type, "aembedding");
    assert!(!pre_call.payload.is_error);
    assert_eq!(pre_call.payload.error, None);
    assert_eq!(pre_call.payload.caller, None);
    assert_eq!(pre_call.payload.event_metadata, None);
    assert_eq!(pre_call.payload.duration, 0.0);
    assert_eq!(pre_call.start_time, harness.clock.base_time());
    assert_eq!(pre_call.end_time, harness.clock.base_time());
}

#[tokio::test]
async fn terminal_success_records_one_event() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    let terminal = &events[1];
    assert_eq!(terminal.call_id, "call-1");
    assert_eq!(terminal.payload.service, ServiceType::Litellm);
    assert_eq!(terminal.payload.call_type, "acompletion");
    assert!(!terminal.payload.is_error);
    assert_eq!(terminal.payload.error, None);
    assert_eq!(terminal.payload.duration, 1.0);
    assert_eq!(terminal.start_time, harness.clock.base_time());
    assert_eq!(
        terminal.end_time,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:01+00:00").unwrap()
    );
}

// A clock that moves in three-second steps proves the duration is measured, not a fixed value.
#[tokio::test]
async fn terminal_duration_comes_from_the_clock() {
    let sink = Arc::new(RecordingSink::default());
    let pipeline = Arc::new(
        ServicePipeline::start(PipelineConfig {
            settings: WorkerSettings::default(),
            clock: SteppingClock::with_step(3),
            sink: Arc::clone(&sink),
            registry: Arc::new(CallbackRegistry::new(16)),
        })
        .unwrap(),
    );
    let mut headers = headers_with("call-1");
    let call = pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    pipeline.worker.flush().await;
    assert_eq!(sink.recorded()[1].payload.duration, 3.0);
}

#[tokio::test]
async fn terminal_failure_records_the_error_text() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Failed(
        "the upstream call failed with HTTP 500".to_owned(),
    ));
    harness.pipeline.worker.flush().await;
    let terminal = &harness.sink.recorded()[1];
    assert!(terminal.payload.is_error);
    assert_eq!(
        terminal.payload.error.as_deref(),
        Some("the upstream call failed with HTTP 500")
    );
    assert_eq!(terminal.payload.duration, 1.0);
}

#[tokio::test]
async fn dropping_a_call_records_one_cancellation() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    drop(call);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert!(events[1].payload.is_error);
    assert_eq!(events[1].payload.error.as_deref(), Some(CANCELLED_ERROR));
}

#[tokio::test]
async fn a_finished_call_records_no_second_event() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    harness.pipeline.worker.flush().await;
    assert_eq!(harness.sink.recorded().len(), 2);
}

#[tokio::test]
async fn stream_end_records_success() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Ok(2)];
    let mut body = call.watch(stream::iter(items), StatusCode::OK);
    while body.next().await.is_some() {}
    drop(body);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert!(!events[1].payload.is_error);
    assert_eq!(events[1].payload.error, None);
}

#[tokio::test]
async fn stream_error_records_the_fixed_failure() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Err("upstream socket closed")];
    let mut body = call.watch(stream::iter(items), StatusCode::OK);
    while body.next().await.is_some() {}
    harness.pipeline.worker.flush().await;
    let terminal = &harness.sink.recorded()[1];
    assert!(terminal.payload.is_error);
    assert_eq!(terminal.payload.error.as_deref(), Some(STREAM_FAILED_ERROR));
    assert!(!format!("{terminal:?}").contains("upstream socket closed"));
}

#[tokio::test]
async fn stream_status_failure_records_the_status() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1)];
    let mut body = call.watch(stream::iter(items), StatusCode::INTERNAL_SERVER_ERROR);
    while body.next().await.is_some() {}
    harness.pipeline.worker.flush().await;
    let terminal = &harness.sink.recorded()[1];
    assert!(terminal.payload.is_error);
    assert_eq!(
        terminal.payload.error.as_deref(),
        Some("the upstream call failed with HTTP 500")
    );
}

#[tokio::test]
async fn dropping_a_stream_records_cancellation() {
    let harness = harness();
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Ok(2)];
    let mut body = call.watch(stream::iter(items), StatusCode::OK);
    assert!(body.next().await.is_some());
    drop(body);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert!(events[1].payload.is_error);
    assert_eq!(events[1].payload.error.as_deref(), Some(CANCELLED_ERROR));
}

#[rstest]
#[case("/v1/chat/completions", Some(ServiceKind::Chat))]
#[case("/chat/completions", Some(ServiceKind::Chat))]
#[case("/v1/embeddings", Some(ServiceKind::Embedding))]
#[case("/embeddings", Some(ServiceKind::Embedding))]
#[case("/v1/completions", None)]
#[case("/v1/chat/completions/extra", None)]
#[case("/", None)]
fn service_kinds_come_from_the_request_path(
    #[case] path: &str,
    #[case] expected: Option<ServiceKind>,
) {
    assert_eq!(ServiceKind::from_path(path), expected);
}

#[test]
fn service_types_round_trip_their_wire_names() {
    for service in ServiceType::ALL {
        assert_eq!(
            serde_json::to_string(&service).unwrap(),
            format!("\"{}\"", service.as_str())
        );
    }
}

#[test]
fn metric_sets_follow_the_service_config() {
    let gauge: &[ServiceMetric] = &[ServiceMetric::Gauge];
    let default: &[ServiceMetric] = &[ServiceMetric::Counter, ServiceMetric::Histogram];
    let gauges = [
        ServiceType::PodLockManager,
        ServiceType::InMemoryDailySpendUpdateQueue,
        ServiceType::RedisDailySpendUpdateQueue,
        ServiceType::RedisDailyEndUserSpendUpdateQueue,
        ServiceType::RedisDailyAgentSpendUpdateQueue,
        ServiceType::InMemorySpendUpdateQueue,
        ServiceType::RedisSpendUpdateQueue,
    ];
    for service in ServiceType::ALL {
        let expected = if gauges.contains(&service) {
            gauge
        } else {
            default
        };
        assert_eq!(service.metrics(), expected, "{service:?}");
    }
}

#[test]
fn system_clock_reads_the_wall_clock() {
    let now = SystemClock.now();
    let wall = Utc::now().fixed_offset();
    assert!(
        (wall - now).num_seconds().abs() <= 5,
        "the system clock read {now}, and the wall clock is at {wall}"
    );
}

#[test]
fn delivery_names_the_service_event_callback() {
    let base = DateTime::parse_from_rfc3339(NOW).unwrap();
    let delivery = Delivery {
        sink: Arc::new(RecordingSink::default()),
        registry: Arc::new(CallbackRegistry::new(1)),
        event: ServiceEvent {
            call_id: "call-1".to_owned(),
            payload: ServiceLoggerPayload {
                is_error: false,
                error: None,
                service: ServiceType::ProxyPreCall,
                duration: 0.0,
                call_type: "acompletion".to_owned(),
                caller: None,
                event_metadata: None,
            },
            start_time: base,
            end_time: base,
        },
    };
    assert_eq!(Callback::name(&delivery), "service_event");
}

#[tokio::test]
async fn a_failing_sink_records_no_second_event() {
    let clock = SteppingClock::new();
    let pipeline = Arc::new(
        ServicePipeline::start(PipelineConfig {
            settings: WorkerSettings::default(),
            clock,
            sink: Arc::new(FailingSink),
            registry: Arc::new(CallbackRegistry::new(64)),
        })
        .unwrap(),
    );
    let mut headers = headers_with("call-1");
    let call = pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    let report = pipeline.worker.flush().await;
    assert_eq!(report.accepted, 2);
    assert_eq!(report.failed, 2);
    assert_eq!(pipeline.worker.stats().accepted, 2);
}

#[tokio::test]
async fn registered_service_callbacks_hear_each_event() {
    let (sender, mut receiver) = mpsc::channel(8);
    let mut registry = CallbackRegistry::new(16);
    let registration = registry.register(
        CallbackIdentity {
            class_name: "recorder".to_owned(),
            attributes: IndexMap::new(),
        },
        CallbackSink::Channel(sender),
        CallbackList::Service,
    );
    assert_eq!(registration, Registration::Registered);
    let harness = harness_with(
        Arc::new(RecordingSink::default()),
        registry,
        WorkerSettings::default(),
    );
    let mut headers = headers_with("call-1");
    let call = harness.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    harness.pipeline.worker.flush().await;
    assert_eq!(receiver.try_recv().unwrap().phase, CallbackPhase::Service);
    assert_eq!(receiver.try_recv().unwrap().phase, CallbackPhase::Service);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn a_zero_queue_size_keeps_the_pipeline_from_starting() {
    let error = ServicePipeline::start(PipelineConfig {
        settings: WorkerSettings {
            max_queue_size: 0,
            ..WorkerSettings::default()
        },
        clock: SteppingClock::new(),
        sink: Arc::new(RecordingSink::default()),
        registry: Arc::new(CallbackRegistry::new(16)),
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        PipelineError::Worker(WorkerError::ZeroQueueSize)
    ));
}

#[tokio::test]
async fn wiring_worker_rejects_a_zero_concurrency() {
    let settings = ServiceCallbackSettings {
        concurrency: 0,
        ..ServiceCallbackSettings::default()
    };
    let error = LoggingWiring::start(
        &settings,
        SteppingClock::new(),
        Arc::new(RecordingSink::default()),
        CallbackRegistry::new(16),
        Arc::new(ChatStreamAssembler),
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        PipelineError::Worker(WorkerError::ZeroConcurrency)
    ));
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log lines are UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn wiring_applies_the_resolved_queue_limit() {
    let settings = ServiceCallbackSettings {
        max_queue_size: 1,
        ..ServiceCallbackSettings::default()
    };
    let sink = Arc::new(RecordingSink::default());
    let wiring = LoggingWiring::start(
        &settings,
        SteppingClock::new(),
        Arc::clone(&sink),
        CallbackRegistry::new(16),
        Arc::new(ChatStreamAssembler),
    )
    .unwrap();
    let mut headers = headers_with("call-1");
    let call = wiring.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    // the queue holds one event, so the two synchronous enqueues admit one and reject one
    let report = wiring.pipeline.stop().await;
    assert_eq!(report.accepted, 1);
    assert_eq!(report.completed, 1);
    assert_eq!(sink.recorded().len(), 1);
}

#[tokio::test]
async fn wiring_reports_callbacks_that_are_not_installed() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    let settings = ServiceCallbackSettings {
        service_callbacks: vec!["datadog".to_owned()],
        ..ServiceCallbackSettings::default()
    };
    let wiring = with_default(subscriber, || {
        LoggingWiring::start(
            &settings,
            SteppingClock::new(),
            Arc::new(RecordingSink::default()),
            CallbackRegistry::new(16),
            Arc::new(ChatStreamAssembler),
        )
    })
    .unwrap();
    let reported = captured.text();
    assert!(
        reported.contains("service callback is not installed"),
        "the wiring did not report the uninstalled callback: {reported}"
    );
    drop(wiring);
}

#[tokio::test]
async fn wiring_starts_the_worker_from_the_resolved_settings() {
    let settings = ServiceCallbackSettings {
        service_callbacks: vec!["datadog".to_owned()],
        log_level: LogLevel::Info,
        json_logs: true,
        max_queue_size: 8,
        concurrency: 2,
        event_timeout: Duration::from_secs(5),
    };
    let sink = Arc::new(RecordingSink::default());
    let wiring = LoggingWiring::start(
        &settings,
        SteppingClock::new(),
        Arc::clone(&sink),
        CallbackRegistry::new(16),
        Arc::new(ChatStreamAssembler),
    )
    .unwrap();
    assert_eq!(wiring.log_level, LogLevel::Info);
    assert!(wiring.json_logs);
    assert_eq!(
        wiring.worker_settings,
        WorkerSettings {
            max_queue_size: 8,
            concurrency: 2,
            event_timeout: Duration::from_secs(5),
            ..WorkerSettings::default()
        }
    );
    let mut headers = headers_with("call-1");
    let call = wiring.pipeline.begin(ServiceKind::Chat, &mut headers);
    call.finish(TerminalOutcome::Succeeded);
    let report = wiring.pipeline.stop().await;
    assert_eq!(report.accepted, 2);
    assert_eq!(report.completed, 2);
    assert!(report.drained);
    assert_eq!(sink.recorded().len(), 2);
}
