#![cfg(test)]

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use tokio_util::task::TaskTracker;

use crate::database_failures::deadline::{DbLookupError, LookupRequest};
use crate::database_failures::gate::{DbLookupStallTracker, SystemClock};
use crate::database_failures::metrics::{
    DbMetricCall, DbMetricError, DbMetricFuture, DbServiceEvent, DbServiceEventSink,
    DbServiceFailureEvent, ProxySpanResolver, log_db_metrics,
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

pub(super) const fn metric_call(function_name: &str) -> DbMetricCall<'_> {
    DbMetricCall {
        function_name,
        parent_otel_span: None,
        table_name: None,
        second_positional: None,
    }
}

pub(super) async fn run_metric<Operation, Output>(
    operation: Operation,
    call: &DbMetricCall<'_>,
    sink: &Arc<RecordingSink>,
    resolver: &RecordingResolver,
    tasks: &TaskTracker,
) -> Result<Output, DbMetricError<SinkFault>>
where
    Operation: Future<Output = Result<Output, DbLookupError>> + Send,
{
    log_db_metrics(operation, call, Arc::clone(sink), resolver, tasks).await
}
