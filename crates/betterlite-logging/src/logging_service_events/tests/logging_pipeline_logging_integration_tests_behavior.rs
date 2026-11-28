use std::convert::Infallible;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use axum::routing::post;
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use futures_util::stream::{self, StreamExt as _};
use http::StatusCode;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tower::ServiceExt as _;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_config::sections::service_callback::{
    DEFAULT_CONCURRENCY as SETTINGS_CONCURRENCY, DEFAULT_EVENT_TIMEOUT as SETTINGS_EVENT_TIMEOUT,
    DEFAULT_LOG_LEVEL, DEFAULT_MAX_QUEUE_SIZE as SETTINGS_QUEUE_SIZE, LogLevelError,
    SettingsValues as ServiceCallbackValues,
};
use betterlite_db::database_diagnostics::level::LogLevel;
use betterlite_error::normalize::Normalizer;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_service_events::clock::SystemClock;
use crate::logging_service_events::middleware::{ServiceEventState, service_events};
use crate::logging_service_events::pipeline::{PipelineConfig, PipelineError, ServicePipeline};
use crate::logging_service_events::sink::{ServiceEventError, ServiceEventSink, TracingSink};
use crate::logging_service_events::stream::{STREAM_FAILED_ERROR, ServiceEventStream, failure_for};
use crate::logging_service_events::types::{
    CANCELLED_ERROR, ServiceEvent, ServiceEventMetadata, ServiceKind, ServiceLoggerPayload,
    ServiceMetric, ServiceType, wire_names,
};
use crate::logging_service_events::wiring::{LoggingInputs, LoggingWiring, SuccessInputs};
use crate::logging_service_events::worker_link::Delivery;
use crate::logging_streams::assembler::ChatStreamAssembler;
use crate::logging_success::ports::SuccessClock;
use crate::payload::builder::PayloadBuilderConfig;
use crate::payload::deps::{PayloadSettings, PayloadSources, TagConfig};
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::{CallbackList, CallbackPhase};
use crate::registry::sink::CallbackSink;
use crate::registry::{CallbackRegistry, Registration};
use crate::worker::{
    Callback, DEFAULT_CONCURRENCY, DEFAULT_EVENT_TIMEOUT, DEFAULT_SHUTDOWN_DRAIN,
    DEFAULT_TIMEOUT_SUMMARY_WINDOW, WorkerError, WorkerSettings,
};

const NOW: &str = "2026-01-01T00:00:00+00:00";
const SECRET: &str = "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// Two clock reads per call keep the duration at one step and both timestamps predictable.
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
        let step = i64::from(self.reads.fetch_add(1, Ordering::SeqCst)).saturating_mul(self.step);
        TimeDelta::try_seconds(step)
            .and_then(|delta| self.base.checked_add_signed(delta))
            .unwrap_or(self.base)
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
    let clock_source: Arc<dyn SuccessClock> = Arc::<SteppingClock>::clone(&clock);
    let sink_source: Arc<dyn ServiceEventSink> = Arc::<RecordingSink>::clone(&sink);
    let pipeline = Arc::new(
        ServicePipeline::start(PipelineConfig {
            settings,
            clock: clock_source,
            sink: sink_source,
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

#[tokio::test]
async fn begin_redacts_a_secret_call_id() {
    let harness = harness();
    let call = harness.pipeline.begin(ServiceKind::Chat, SECRET);
    call.finish(None);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events[0].call_id, "REDACTED");
    assert_eq!(events[1].call_id, "REDACTED");
}

#[tokio::test]
async fn pre_call_event_carries_the_call_identity() {
    let harness = harness();
    drop(harness.pipeline.begin(ServiceKind::Embedding, "call-1"));
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
    assert_eq!(pre_call.payload.duration.to_bits(), 0.0_f64.to_bits());
    assert_eq!(pre_call.start_time, harness.clock.base_time());
    assert_eq!(pre_call.end_time, harness.clock.base_time());
}

#[tokio::test]
async fn terminal_success_records_one_event() {
    let harness = harness();
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    let terminal = &events[1];
    assert_eq!(terminal.call_id, "call-1");
    assert_eq!(terminal.payload.service, ServiceType::Litellm);
    assert_eq!(terminal.payload.call_type, "acompletion");
    assert!(!terminal.payload.is_error);
    assert_eq!(terminal.payload.error, None);
    assert_eq!(terminal.payload.duration.to_bits(), 1.0_f64.to_bits());
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
    let sink_source: Arc<dyn ServiceEventSink> = Arc::<RecordingSink>::clone(&sink);
    let pipeline = Arc::new(
        ServicePipeline::start(PipelineConfig {
            settings: WorkerSettings::default(),
            clock: SteppingClock::with_step(3),
            sink: sink_source,
            registry: Arc::new(CallbackRegistry::new(16)),
        })
        .unwrap(),
    );
    let call = pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
    pipeline.worker.flush().await;
    assert_eq!(
        sink.recorded()[1].payload.duration.to_bits(),
        3.0_f64.to_bits()
    );
}

#[tokio::test]
async fn terminal_failure_records_the_error_text() {
    let harness = harness();
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(Some("the upstream call failed with HTTP 500".to_owned()));
    harness.pipeline.worker.flush().await;
    let terminal = &harness.sink.recorded()[1];
    assert!(terminal.payload.is_error);
    assert_eq!(
        terminal.payload.error.as_deref(),
        Some("the upstream call failed with HTTP 500")
    );
    assert_eq!(terminal.payload.duration.to_bits(), 1.0_f64.to_bits());
}

#[tokio::test]
async fn dropping_a_call_records_one_cancellation() {
    let harness = harness();
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
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
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
    harness.pipeline.worker.flush().await;
    assert_eq!(harness.sink.recorded().len(), 2);
}

#[tokio::test]
async fn stream_end_records_success() {
    let harness = harness();
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Ok(2)];
    let mut body = ServiceEventStream::new(stream::iter(items), call, failure_for(StatusCode::OK));
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
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Err("upstream socket closed")];
    let mut body = ServiceEventStream::new(stream::iter(items), call, failure_for(StatusCode::OK));
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
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1)];
    let mut body = ServiceEventStream::new(
        stream::iter(items),
        call,
        failure_for(StatusCode::INTERNAL_SERVER_ERROR),
    );
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
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    let items: Vec<Result<u8, &'static str>> = vec![Ok(1), Ok(2)];
    let mut body = ServiceEventStream::new(stream::iter(items), call, failure_for(StatusCode::OK));
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
#[case("/v1/completions", Some(ServiceKind::TextCompletion))]
#[case("/completions", Some(ServiceKind::TextCompletion))]
#[case(
    "/engines/gpt-3.5-turbo-instruct/completions",
    Some(ServiceKind::TextCompletion)
)]
#[case(
    "/openai/deployments/instruct/completions",
    Some(ServiceKind::TextCompletion)
)]
#[case("/v1/embeddings", Some(ServiceKind::Embedding))]
#[case("/embeddings", Some(ServiceKind::Embedding))]
#[case("/v1/chat/completions/extra", None)]
#[case("/", None)]
fn service_kinds_come_from_the_request_path(
    #[case] path: &str,
    #[case] expected: Option<ServiceKind>,
) {
    assert_eq!(ServiceKind::from_path(path), expected);
}

#[rstest]
#[case::redis(ServiceType::Redis, "redis")]
#[case::postgres(ServiceType::Postgres, "postgres")]
#[case::batch_write_to_db(ServiceType::BatchWriteToDb, "batch_write_to_db")]
#[case::reset_budget_job(ServiceType::ResetBudgetJob, "reset_budget_job")]
#[case::litellm(ServiceType::Litellm, "self")]
#[case::router(ServiceType::Router, "router")]
#[case::auth(ServiceType::Auth, "auth")]
#[case::proxy_pre_call(ServiceType::ProxyPreCall, "proxy_pre_call")]
#[case::pod_lock_manager(ServiceType::PodLockManager, "pod_lock_manager")]
#[case::in_memory_daily(
    ServiceType::InMemoryDailySpendUpdateQueue,
    "in_memory_daily_spend_update_queue"
)]
#[case::redis_daily(
    ServiceType::RedisDailySpendUpdateQueue,
    "redis_daily_spend_update_queue"
)]
#[case::redis_daily_end_user(
    ServiceType::RedisDailyEndUserQueue,
    "redis_daily_end_user_spend_update_queue"
)]
#[case::redis_daily_org(
    ServiceType::RedisDailyOrgSpendUpdateQueue,
    "redis_daily_org_spend_update_queue"
)]
#[case::redis_daily_team(
    ServiceType::RedisDailyTeamSpendUpdateQueue,
    "redis_daily_team_spend_update_queue"
)]
#[case::redis_daily_agent(
    ServiceType::RedisDailyAgentQueue,
    "redis_daily_agent_spend_update_queue"
)]
#[case::redis_daily_tag(
    ServiceType::RedisDailyTagSpendUpdateQueue,
    "redis_daily_tag_spend_update_queue"
)]
#[case::in_memory_spend(ServiceType::InMemorySpendUpdateQueue, "in_memory_spend_update_queue")]
#[case::redis_spend(ServiceType::RedisSpendUpdateQueue, "redis_spend_update_queue")]
#[case::redis_window(
    ServiceType::RedisWindowSpendUpdateQueue,
    "redis_window_spend_update_queue"
)]
fn service_types_round_trip_their_wire_names(#[case] service: ServiceType, #[case] wire: &str) {
    assert_eq!(wire_names::as_str(service), wire);
    assert_eq!(
        serde_json::to_string(&service).unwrap(),
        format!("\"{wire}\"")
    );
}

#[rstest]
#[case::redis(ServiceType::Redis, false)]
#[case::postgres(ServiceType::Postgres, false)]
#[case::batch_write_to_db(ServiceType::BatchWriteToDb, false)]
#[case::reset_budget_job(ServiceType::ResetBudgetJob, false)]
#[case::litellm(ServiceType::Litellm, false)]
#[case::router(ServiceType::Router, false)]
#[case::auth(ServiceType::Auth, false)]
#[case::proxy_pre_call(ServiceType::ProxyPreCall, false)]
#[case::pod_lock_manager(ServiceType::PodLockManager, true)]
#[case::in_memory_daily(ServiceType::InMemoryDailySpendUpdateQueue, true)]
#[case::redis_daily(ServiceType::RedisDailySpendUpdateQueue, true)]
#[case::redis_daily_end_user(ServiceType::RedisDailyEndUserQueue, true)]
#[case::redis_daily_org(ServiceType::RedisDailyOrgSpendUpdateQueue, false)]
#[case::redis_daily_team(ServiceType::RedisDailyTeamSpendUpdateQueue, false)]
#[case::redis_daily_agent(ServiceType::RedisDailyAgentQueue, true)]
#[case::redis_daily_tag(ServiceType::RedisDailyTagSpendUpdateQueue, false)]
#[case::in_memory_spend(ServiceType::InMemorySpendUpdateQueue, true)]
#[case::redis_spend(ServiceType::RedisSpendUpdateQueue, true)]
#[case::redis_window(ServiceType::RedisWindowSpendUpdateQueue, false)]
fn metric_sets_follow_the_service_config(#[case] service: ServiceType, #[case] gauge: bool) {
    let expected: &[ServiceMetric] = if gauge {
        &[ServiceMetric::Gauge]
    } else {
        &[ServiceMetric::Counter, ServiceMetric::Histogram]
    };
    assert_eq!(service.metrics(), expected, "{service:?}");
}

#[test]
fn service_event_metadata_carries_only_the_fields_it_sets() {
    let filled = ServiceEventMetadata {
        gauge_labels: Some("worker".to_owned()),
        gauge_value: Some(2.5_f64),
    };
    assert_eq!(
        serde_json::to_string(&filled).unwrap(),
        r#"{"gauge_labels":"worker","gauge_value":2.5}"#
    );
    let empty = ServiceEventMetadata::default();
    assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
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
    let call = pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
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
    let call = harness.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
    harness.pipeline.worker.flush().await;
    assert_eq!(receiver.try_recv().unwrap().phase, CallbackPhase::Service);
    assert_eq!(receiver.try_recv().unwrap().phase, CallbackPhase::Service);
    assert_eq!(receiver.try_recv().unwrap_err(), TryRecvError::Empty);
}

#[tokio::test]
async fn a_zero_queue_size_keeps_the_pipeline_from_starting() {
    let error = ServicePipeline::start(PipelineConfig {
        settings: WorkerSettings {
            max_queue_size: 0,
            concurrency: DEFAULT_CONCURRENCY,
            event_timeout: DEFAULT_EVENT_TIMEOUT,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
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

fn success_inputs() -> SuccessInputs {
    SuccessInputs {
        payload: PayloadBuilderConfig {
            sources: Arc::new(PayloadSources::default()),
            settings: PayloadSettings {
                tag_config: TagConfig::default(),
                request_correlation_in_logs: false,
                log_client_error_tracebacks: false,
            },
            redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
            normalizer: Arc::new(Normalizer::new().unwrap()),
            truncator: Base64Truncator::new(4_096).unwrap(),
            privacy: Settings::default(),
        },
        excluded_fields: Vec::new(),
        known_callback: Arc::new(|_name: &str| false),
    }
}

#[tokio::test]
async fn wiring_worker_rejects_a_zero_concurrency() {
    let settings = Box::new(ServiceCallbackValues {
        service_callbacks: Vec::new(),
        log_level: DEFAULT_LOG_LEVEL.to_owned(),
        json_logs: false,
        max_queue_size: SETTINGS_QUEUE_SIZE,
        concurrency: 0,
        event_timeout: SETTINGS_EVENT_TIMEOUT,
    });
    let error = LoggingWiring::start(LoggingInputs {
        settings: &settings,
        clock: SteppingClock::new(),
        sink: Arc::new(RecordingSink::default()),
        registry: CallbackRegistry::new(16),
        stream_assembler: Arc::new(ChatStreamAssembler),
        success: success_inputs(),
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        PipelineError::Worker(WorkerError::ZeroConcurrency)
    ));
}

#[tokio::test]
async fn wiring_rejects_an_unknown_log_level() {
    let settings = Box::new(ServiceCallbackValues {
        service_callbacks: Vec::new(),
        log_level: "verbose".to_owned(),
        json_logs: false,
        max_queue_size: SETTINGS_QUEUE_SIZE,
        concurrency: SETTINGS_CONCURRENCY,
        event_timeout: SETTINGS_EVENT_TIMEOUT,
    });
    let error = LoggingWiring::start(LoggingInputs {
        settings: &settings,
        clock: SteppingClock::new(),
        sink: Arc::new(RecordingSink::default()),
        registry: CallbackRegistry::new(16),
        stream_assembler: Arc::new(ChatStreamAssembler),
        success: success_inputs(),
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        PipelineError::LogLevel(LogLevelError::Unknown { .. })
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
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
    let settings = Box::new(ServiceCallbackValues {
        service_callbacks: Vec::new(),
        log_level: DEFAULT_LOG_LEVEL.to_owned(),
        json_logs: false,
        max_queue_size: 1,
        concurrency: SETTINGS_CONCURRENCY,
        event_timeout: SETTINGS_EVENT_TIMEOUT,
    });
    let sink = Arc::new(RecordingSink::default());
    let sink_source: Arc<dyn ServiceEventSink> = Arc::<RecordingSink>::clone(&sink);
    let wiring = LoggingWiring::start(LoggingInputs {
        settings: &settings,
        clock: SteppingClock::new(),
        sink: sink_source,
        registry: CallbackRegistry::new(16),
        stream_assembler: Arc::new(ChatStreamAssembler),
        success: success_inputs(),
    })
    .unwrap();
    let call = wiring.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
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
    let settings = Box::new(ServiceCallbackValues {
        service_callbacks: vec!["datadog".to_owned()],
        log_level: DEFAULT_LOG_LEVEL.to_owned(),
        json_logs: false,
        max_queue_size: SETTINGS_QUEUE_SIZE,
        concurrency: SETTINGS_CONCURRENCY,
        event_timeout: SETTINGS_EVENT_TIMEOUT,
    });
    let wiring = with_default(subscriber, || {
        LoggingWiring::start(LoggingInputs {
            settings: &settings,
            clock: SteppingClock::new(),
            sink: Arc::new(RecordingSink::default()),
            registry: CallbackRegistry::new(16),
            stream_assembler: Arc::new(ChatStreamAssembler),
            success: success_inputs(),
        })
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
    let settings = Box::new(ServiceCallbackValues {
        service_callbacks: vec!["datadog".to_owned()],
        log_level: "info".to_owned(),
        json_logs: true,
        max_queue_size: 8,
        concurrency: 2,
        event_timeout: Duration::from_secs(5),
    });
    let sink = Arc::new(RecordingSink::default());
    let sink_source: Arc<dyn ServiceEventSink> = Arc::<RecordingSink>::clone(&sink);
    let wiring = LoggingWiring::start(LoggingInputs {
        settings: &settings,
        clock: SteppingClock::new(),
        sink: sink_source,
        registry: CallbackRegistry::new(16),
        stream_assembler: Arc::new(ChatStreamAssembler),
        success: success_inputs(),
    })
    .unwrap();
    assert_eq!(wiring.log_level, LogLevel::Info);
    assert!(wiring.json_logs);
    assert_eq!(
        wiring.worker_settings,
        WorkerSettings {
            max_queue_size: 8,
            concurrency: 2,
            event_timeout: Duration::from_secs(5),
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
        }
    );
    let call = wiring.pipeline.begin(ServiceKind::Chat, "call-1");
    call.finish(None);
    let report = wiring.pipeline.stop().await;
    assert_eq!(report.accepted, 2);
    assert_eq!(report.completed, 2);
    assert!(report.drained);
    assert_eq!(sink.recorded().len(), 2);
}

// the sink lives in this crate, and cargo-mutants tests only the mutated package
#[test]
fn tracing_sink_logs_the_service_event() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    let base = DateTime::parse_from_rfc3339(NOW).unwrap();
    let event = ServiceEvent {
        call_id: "call-1".to_owned(),
        payload: ServiceLoggerPayload {
            is_error: true,
            error: Some("the upstream call failed".to_owned()),
            service: ServiceType::ProxyPreCall,
            duration: 1.5,
            call_type: "acompletion".to_owned(),
            caller: None,
            event_metadata: None,
        },
        start_time: base,
        end_time: base,
    };
    with_default(subscriber, || TracingSink.emit(&event)).unwrap();
    let logged = captured.text();
    assert!(logged.contains("service event"), "no message: {logged}");
    assert!(logged.contains("call-1"), "no call id: {logged}");
    assert!(logged.contains("proxy_pre_call"), "no service: {logged}");
    assert!(logged.contains("acompletion"), "no call type: {logged}");
    assert!(logged.contains("is_error=true"), "no error flag: {logged}");
    assert!(logged.contains("duration=1.5"), "no duration: {logged}");
    assert!(
        logged.contains("the upstream call failed"),
        "no error text: {logged}"
    );
}

async fn chat_answer(headers: HeaderMap) -> Response {
    let mut response = Response::new(Body::from("{\"id\":\"chat-1\"}"));
    if let Some(value) = headers.get("x-litellm-call-id") {
        response
            .headers_mut()
            .insert("x-seen-call-id", value.clone());
    }
    response
}

async fn streamed_answer() -> Response {
    let chunks = stream::iter([Ok::<_, Infallible>(Bytes::from_static(
        b"{\"id\":\"chat-2\"}",
    ))]);
    Response::new(Body::from_stream(chunks))
}

fn resolve_test_call_id(header: Option<&str>) -> String {
    header.unwrap_or("generated-call").to_owned()
}

// The middleware lives in this crate, and cargo-mutants tests only the mutated package.
fn event_app(pipeline: &Arc<ServicePipeline>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat_answer))
        .route("/v1/embeddings", post(streamed_answer))
        .route("/health", post(chat_answer))
        .layer(from_fn_with_state(
            ServiceEventState {
                pipeline: Arc::clone(pipeline),
                call_id: resolve_test_call_id,
            },
            service_events,
        ))
}

#[tokio::test]
async fn middleware_records_one_pre_call_and_one_terminal_event() {
    let harness = harness();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let response = event_app(&harness.pipeline).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-seen-call-id"], "generated-call");
    let length = response.headers()["content-length"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(length.parse::<usize>().unwrap(), body.len());
    assert_eq!(body, Bytes::from_static(b"{\"id\":\"chat-1\"}"));
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload.service, ServiceType::ProxyPreCall);
    assert_eq!(events[0].call_id, "generated-call");
    assert_eq!(events[1].payload.service, ServiceType::Litellm);
    assert!(!events[1].payload.is_error);
}

#[tokio::test]
async fn middleware_keeps_the_client_call_id() {
    let harness = harness();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("x-litellm-call-id", "call-7")
        .body(Body::empty())
        .unwrap();
    let response = event_app(&harness.pipeline).oneshot(request).await.unwrap();
    assert_eq!(response.headers()["x-seen-call-id"], "call-7");
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].call_id, "call-7");
    assert_eq!(events[1].call_id, "call-7");
}

#[tokio::test]
async fn middleware_settles_a_streamed_body_when_the_stream_ends() {
    let harness = harness();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
        .body(Body::empty())
        .unwrap();
    let response = event_app(&harness.pipeline).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body, Bytes::from_static(b"{\"id\":\"chat-2\"}"));
    harness.pipeline.worker.flush().await;
    let events = harness.sink.recorded();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].payload.call_type, "aembedding");
    assert!(!events[1].payload.is_error);
}

#[tokio::test]
async fn middleware_leaves_a_foreign_path_untracked() {
    let harness = harness();
    let request = Request::builder()
        .method("POST")
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let response = event_app(&harness.pipeline).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key("x-seen-call-id"));
    harness.pipeline.worker.flush().await;
    assert!(harness.sink.recorded().is_empty());
}
