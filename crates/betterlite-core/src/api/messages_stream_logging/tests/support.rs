#![cfg(test)]

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, Duration as Seconds, FixedOffset};
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};

use super::super::seams::{
    ChunkProcessorCall, LoggingFuture, LoggingJob, ReportedFailure, RouteStreamingLogging,
    StreamSeams,
};
use super::super::slots::{DeferredCallback, StreamSlots};
use super::super::{MessagesStreamLogging, StreamDeps};

pub(super) fn failure(kind: &str, message: &str) -> ReportedFailure {
    ReportedFailure {
        kind: kind.to_owned(),
        message: message.to_owned(),
    }
}

pub(super) fn base_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

// Each reading is one second after the one before, so t0, t1 and t2 are the first three
pub(super) fn at(seconds: i64) -> DateTime<FixedOffset> {
    let step = Seconds::try_seconds(seconds).expect("the step is small");
    base_time().checked_add_signed(step).expect("the time fits")
}

pub(super) struct StepClock {
    reads: Mutex<i64>,
}

impl SuccessClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let now = at(*reads);
        *reads = reads.saturating_add(1);
        now
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object() -> Arc<StreamLoggingObject> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: base_time(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds");
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    })
}

// Records every seam call; the logging future it returns counts how often it ran
pub(super) struct Recorder {
    pub(super) routed: Mutex<Vec<RouteStreamingLogging>>,
    pub(super) processed: Mutex<Vec<ChunkProcessorCall<String>>>,
    pub(super) ran: Arc<AtomicUsize>,
    pub(super) route_failure: Option<ReportedFailure>,
}

impl StreamSeams for Recorder {
    type Response = String;
    type Stream = String;

    fn route_streaming_logging(
        &self,
        call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        if let Some(failure) = &self.route_failure {
            return Err(failure.clone());
        }
        self.routed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let ran = Arc::clone(&self.ran);
        Ok(Box::pin(async move {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<String>) -> String {
        let relayed = format!("processed {}", call.response);
        self.processed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        relayed
    }
}

pub(super) struct Harness {
    pub(super) seams: Arc<Recorder>,
    pub(super) worker: Arc<LoggingWorker<LoggingJob>>,
    pub(super) slots: Arc<StreamSlots>,
    pub(super) handler: Arc<PassThroughEndpointLogging>,
    pub(super) logging_obj: Arc<StreamLoggingObject>,
    pub(super) request_body: IndexMap<String, Json>,
    pub(super) stream: MessagesStreamLogging<Recorder>,
}

impl Harness {
    pub(super) fn new(route_failure: Option<ReportedFailure>) -> Self {
        let seams = Arc::new(Recorder {
            routed: Mutex::new(Vec::new()),
            processed: Mutex::new(Vec::new()),
            ran: Arc::new(AtomicUsize::new(0)),
            route_failure,
        });
        let worker =
            Arc::new(LoggingWorker::start(WorkerSettings::default()).expect("the worker starts"));
        let slots = Arc::new(StreamSlots::default());
        let handler = Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        });
        let logging_obj = logging_object();
        let request_body = IndexMap::from([("model".to_owned(), Json::Text("claude".to_owned()))]);
        let deps = StreamDeps {
            seams: Arc::clone(&seams),
            worker: Arc::clone(&worker),
            slots: Arc::clone(&slots),
            clock: Arc::new(StepClock {
                reads: Mutex::new(0),
            }),
            success_handler: Arc::clone(&handler),
        };
        let stream =
            MessagesStreamLogging::new(deps, Arc::clone(&logging_obj), request_body.clone());
        Self {
            seams,
            worker,
            slots,
            handler,
            logging_obj,
            request_body,
            stream,
        }
    }

    pub(super) fn ran(&self) -> usize {
        self.seams.ran.load(Ordering::SeqCst)
    }
}

// Python's callback: it records how many arguments it got, then runs the parked logging itself
pub(super) fn deferred_callback(calls: &Arc<Mutex<Vec<usize>>>) -> DeferredCallback {
    let calls = Arc::clone(calls);
    Box::new(move |args: Vec<LoggingFuture>| -> LoggingFuture {
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(args.len());
        Box::pin(async move {
            for parked in args {
                parked.await?;
            }
            Ok(())
        })
    })
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is text")
    }
}

impl io::Write for Captured {
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

pub(super) fn capture_warnings() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
