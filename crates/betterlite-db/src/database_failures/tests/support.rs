#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::otel::model::metadata::wire::callback::Params;
use betterlite_logging::otel::model::metadata::wire::decode;
use betterlite_logging::otel::model::metadata::wire::payload::Metadata;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::parent_span::{
    ParentSpanSources, parent_otel_span_from_kwargs,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use serde::Deserialize;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::database_failures::deadline::{DbLookupError, LookupRequest};
use crate::database_failures::gate::{DbLookupStallTracker, SystemClock};
use crate::database_failures::metrics::{
    DbMetricCall, DbMetricContext, DbMetricError, DbMetricFuture, DbServiceEvent,
    DbServiceEventSink, DbServiceFailureEvent, ProxySpanResolver, SystemWallClock, WallClock,
    log_db_metrics,
};
use crate::database_faults::{DbError, ErrorType};

pub(super) fn database_error(kind: ErrorType, message: &str) -> DbError {
    DbError {
        error_type: kind,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub(super) const fn stall_tracker() -> DbLookupStallTracker {
    DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    }
}

pub(super) const fn request<'request, Clock>(
    name: &'request str,
    deadline_seconds: Option<f64>,
    default_deadline_seconds: f64,
    tracker: &'request DbLookupStallTracker<Clock>,
) -> LookupRequest<'request, Clock> {
    LookupRequest {
        name,
        deadline_seconds,
        default_deadline_seconds,
        tracker,
    }
}

pub(super) async fn drain(tasks: &TaskTracker) {
    tasks.close();
    tasks.wait().await;
}

pub(super) fn assert_deadline(error: &DbLookupError, lookup: &str, seconds: f64) {
    let DbLookupError::Deadline(exceeded) = error else {
        panic!("the lookup must fail with a deadline, got {error:?}");
    };
    assert_eq!(exceeded.lookup, lookup);
    assert_eq!(exceeded.deadline_seconds.to_bits(), seconds.to_bits());
}

pub(super) fn span(byte: u8) -> SpanContext {
    SpanContext::new(
        TraceId::from_bytes([byte; 16]),
        SpanId::from_bytes([byte; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    )
}

#[derive(Debug, thiserror::Error)]
#[error("the sink refused the event")]
pub(super) struct SinkFault;

#[derive(Default)]
pub(super) struct RecordingSink {
    pub(super) successes: Mutex<Vec<DbServiceEvent>>,
    pub(super) failures: Mutex<Vec<DbServiceFailureEvent>>,
    pub(super) refuse_success: AtomicBool,
    pub(super) refuse_failure: AtomicBool,
}

impl DbServiceEventSink for RecordingSink {
    type Failure = SinkFault;

    fn success(&self, event: DbServiceEvent) -> DbMetricFuture<'_, SinkFault> {
        self.successes
            .lock()
            .expect("the recording sink is not poisoned")
            .push(event);
        let refused = self.refuse_success.load(Ordering::SeqCst);
        Box::pin(async move { if refused { Err(SinkFault) } else { Ok(()) } })
    }

    fn failure(&self, event: DbServiceFailureEvent) -> DbMetricFuture<'_, SinkFault> {
        self.failures
            .lock()
            .expect("the recording sink is not poisoned")
            .push(event);
        let refused = self.refuse_failure.load(Ordering::SeqCst);
        Box::pin(async move { if refused { Err(SinkFault) } else { Ok(()) } })
    }
}

pub(super) fn successes(sink: &RecordingSink) -> Vec<DbServiceEvent> {
    sink.successes
        .lock()
        .expect("the recording sink is not poisoned")
        .clone()
}

pub(super) fn failures(sink: &RecordingSink) -> Vec<DbServiceFailureEvent> {
    sink.failures
        .lock()
        .expect("the recording sink is not poisoned")
        .clone()
}

#[derive(Default)]
pub(super) struct RecordingResolver {
    pub(super) span: Option<SpanContext>,
    pub(super) seen: Mutex<Vec<IndexMap<String, Json>>>,
}

impl ProxySpanResolver for RecordingResolver {
    fn parent_span(&self, passed_kwargs: &IndexMap<String, Json>) -> Option<SpanContext> {
        self.seen
            .lock()
            .expect("the recording resolver is not poisoned")
            .push(passed_kwargs.clone());
        self.span.clone()
    }
}

pub(super) struct RecordingWallClock {
    times: Mutex<VecDeque<DateTime<Utc>>>,
}

impl RecordingWallClock {
    pub(super) fn new(times: Vec<DateTime<Utc>>) -> Self {
        Self {
            times: Mutex::new(VecDeque::from(times)),
        }
    }
}

impl WallClock for RecordingWallClock {
    fn now(&self) -> DateTime<Utc> {
        self.times
            .lock()
            .expect("the recording clock is not poisoned")
            .pop_front()
            .expect("the test supplies a time for every read")
    }
}

pub(super) const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";

#[derive(Deserialize)]
struct MetricKwargs {
    metadata: Option<Metadata>,
    litellm_params: Option<Params>,
    litellm_parent_otel_span: Option<OtelModelValue>,
}

pub(super) struct SharedSpanResolver;

impl ProxySpanResolver for SharedSpanResolver {
    fn parent_span(&self, passed_kwargs: &IndexMap<String, Json>) -> Option<SpanContext> {
        let text = serde_json::to_string(passed_kwargs).ok()?;
        let kwargs: MetricKwargs = decode(&text).ok()?;
        parent_otel_span_from_kwargs(&ParentSpanSources {
            metadata: kwargs.metadata.as_ref(),
            params: kwargs.litellm_params.as_ref(),
            span: kwargs.litellm_parent_otel_span.as_ref(),
        })
    }
}

pub(super) const fn metric_call(function_name: &str) -> DbMetricCall<'_> {
    DbMetricCall {
        function_name,
        parent_otel_span: None,
        table_name: None,
        second_positional: None,
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured logs are utf-8")
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

pub(super) async fn run_metric<Operation, Output>(
    operation: Operation,
    call: &DbMetricCall<'_>,
    sink: &Arc<RecordingSink>,
    resolver: &dyn ProxySpanResolver,
    tasks: &TaskTracker,
) -> Result<Output, DbMetricError<SinkFault>>
where
    Operation: Future<Output = Result<Output, DbLookupError>> + Send,
{
    let clock = SystemWallClock;
    let context = DbMetricContext {
        sink: Arc::clone(sink),
        resolver,
        clock: &clock,
        tasks,
    };
    log_db_metrics(operation, call, context).await
}
