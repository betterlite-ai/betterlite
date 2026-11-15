#![cfg(test)]

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::yield_now;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};

use super::super::queue::{ClientQueue, State};
use super::super::registry::StreamRegistry;
use super::super::{
    ClientRelay, MessagesChunkSource, RelayLimits, RelaySetup, StreamFailureCall,
    StreamFailureLogging, start_relay,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, LoggingJob, ReportedFailure, RouteStreamingLogging,
    StreamSeams,
};
use crate::api::messages_stream_logging::slots::{
    DeferredCallback, DetachedFailureHook, Fields, StreamSlots,
};
use crate::api::messages_stream_logging::{SseChunk, StreamDeps};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}({message})")]
pub(super) struct ProviderError {
    pub(super) kind: &'static str,
    pub(super) message: &'static str,
}

impl From<ProviderError> for ReportedFailure {
    fn from(error: ProviderError) -> Self {
        Self {
            kind: error.kind.to_owned(),
            message: error.message.to_owned(),
        }
    }
}

pub(super) enum Command {
    Chunk(SseChunk),
    Fail(ProviderError),
    End,
}

pub(super) struct Script {
    commands: mpsc::Receiver<Command>,
    delivered: Arc<Semaphore>,
}

pub(super) struct ScriptHandle {
    commands: mpsc::Sender<Command>,
    delivered: Arc<Semaphore>,
    pulls: Arc<AtomicUsize>,
}

impl ScriptHandle {
    pub(super) async fn release(&self, chunk: SseChunk) {
        self.send(Command::Chunk(chunk)).await;
    }

    pub(super) async fn fail(&self, error: ProviderError) {
        self.send(Command::Fail(error)).await;
    }

    pub(super) async fn finish(&self) {
        self.send(Command::End).await;
    }

    pub(super) fn try_release(&self, chunk: SseChunk) {
        self.commands
            .try_send(Command::Chunk(chunk))
            .expect("the script channel has room");
    }

    pub(super) async fn await_delivery(&self) {
        let _permit = self.delivered.acquire().await.expect("the pump reports");
    }

    pub(super) fn pulls(&self) -> usize {
        self.pulls.load(Ordering::SeqCst)
    }

    async fn send(&self, command: Command) {
        self.commands.send(command).await.expect("the pump reads");
        self.await_delivery().await;
    }
}

pub(super) struct ScriptedSource {
    script: Script,
    pulls: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl MessagesProviderStream for ScriptedSource {
    type Error = ProviderError;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ProviderError> {
        Ok(None)
    }

    async fn aclose(&mut self) -> Option<Result<(), ProviderError>> {
        self.closed.store(true, Ordering::SeqCst);
        Some(Ok(()))
    }
}

#[async_trait]
impl MessagesChunkSource for ScriptedSource {
    async fn next_sse_chunk(&mut self) -> Result<Option<SseChunk>, ProviderError> {
        let next = self.script.commands.recv().await;
        self.pulls.fetch_add(1, Ordering::SeqCst);
        self.script.delivered.add_permits(1);
        match next {
            Some(Command::Chunk(chunk)) => Ok(Some(chunk)),
            Some(Command::Fail(error)) => Err(error),
            Some(Command::End) | None => Ok(None),
        }
    }
}

pub(super) fn queue_of(capacity: usize) -> Arc<ClientQueue<ProviderError>> {
    Arc::new(ClientQueue {
        capacity,
        state: Mutex::new(State {
            items: VecDeque::new(),
            closed: false,
        }),
        data: Notify::new(),
        space: Notify::new(),
        client_detached: AtomicBool::new(false),
        detach: Notify::new(),
    })
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

pub(super) fn at(seconds: i64) -> DateTime<FixedOffset> {
    let base =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    let step = Seconds::try_seconds(seconds).expect("the step is small");
    base.checked_add_signed(step).expect("the time fits")
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_object() -> Arc<StreamLoggingObject> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: at(0),
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
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

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) routed: Mutex<Vec<RouteStreamingLogging>>,
    pub(super) failures: Mutex<Vec<StreamFailureCall>>,
    pub(super) ran: Arc<AtomicUsize>,
}

impl StreamSeams for Recorder {
    type Response = String;
    type Stream = String;

    fn route_streaming_logging(
        &self,
        call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
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
        format!("processed {}", call.response)
    }
}

#[async_trait]
impl StreamFailureLogging for Recorder {
    async fn schedule_failure_logging(&self, call: StreamFailureCall) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

pub(super) struct Harness {
    pub(super) seams: Arc<Recorder>,
    pub(super) slots: Arc<StreamSlots>,
    pub(super) logging_obj: Arc<StreamLoggingObject>,
    pub(super) worker: Arc<LoggingWorker<LoggingJob>>,
    pub(super) script: ScriptHandle,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) pump: Arc<TaskTracker>,
}

impl Harness {
    pub(super) fn fields(&self) -> MutexGuard<'_, Fields> {
        self.slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn start(
        limits: RelayLimits,
        registry: Arc<StreamRegistry>,
    ) -> (Self, ClientRelay<Recorder, ProviderError>) {
        Self::start_with(limits, registry, ApiLimits::default())
    }

    // A case that drives the relay's own read and item bounds passes them here instead
    pub(super) fn start_with(
        limits: RelayLimits,
        registry: Arc<StreamRegistry>,
        boundaries: ApiLimits,
    ) -> (Self, ClientRelay<Recorder, ProviderError>) {
        let (source, script, closed) = scripted_source();
        let seams = Arc::new(Recorder::default());
        let worker = Arc::new(LoggingWorker::start(WorkerSettings::default()).expect("starts"));
        let slots = Arc::new(StreamSlots::default());
        let logging_obj = logging_object();
        let deps = stream_deps(Arc::clone(&seams), Arc::clone(&worker), Arc::clone(&slots));
        let setup = relay_setup(limits, boundaries, registry, deps, Arc::clone(&logging_obj));
        let handle = start_relay(source, setup);
        let harness = Self {
            seams,
            slots,
            logging_obj,
            worker,
            script,
            closed,
            pump: handle.pump,
        };
        (harness, handle.relay)
    }

    pub(super) fn routed_frames(&self) -> Vec<Vec<u8>> {
        let routed = self
            .seams
            .routed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        routed
            .first()
            .map(|call| call.raw_bytes.clone())
            .unwrap_or_default()
    }

    pub(super) fn billed(&self) -> usize {
        self.seams
            .routed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn ran(&self) -> usize {
        self.seams.ran.load(Ordering::SeqCst)
    }

    pub(super) fn failures(&self) -> Vec<StreamFailureCall> {
        self.seams
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    pub(super) fn completion_start_time(&self) -> Option<DateTime<FixedOffset>> {
        self.logging_obj
            .read(&mut |state| state.completion_start_time)
            .flatten()
    }

    pub(super) fn parked(&self) -> bool {
        self.slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .deferred_args
            .is_some()
    }
}

fn scripted_source() -> (ScriptedSource, ScriptHandle, Arc<AtomicBool>) {
    let (commands, receiver) = mpsc::channel(16);
    let delivered = Arc::new(Semaphore::new(0));
    let pulls = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    let source = ScriptedSource {
        script: Script {
            commands: receiver,
            delivered: Arc::clone(&delivered),
        },
        pulls: Arc::clone(&pulls),
        closed: Arc::clone(&closed),
    };
    let script = ScriptHandle {
        commands,
        delivered,
        pulls,
    };
    (source, script, closed)
}

fn stream_deps(
    seams: Arc<Recorder>,
    worker: Arc<LoggingWorker<LoggingJob>>,
    slots: Arc<StreamSlots>,
) -> StreamDeps<Recorder> {
    StreamDeps {
        seams,
        worker,
        slots,
        clock: Arc::new(StepClock {
            reads: Mutex::new(0),
        }),
        success_handler: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
    }
}

fn relay_setup(
    limits: RelayLimits,
    boundaries: ApiLimits,
    registry: Arc<StreamRegistry>,
    deps: StreamDeps<Recorder>,
    logging_obj: Arc<StreamLoggingObject>,
) -> RelaySetup<Recorder, Recorder> {
    let failure_logging = Arc::clone(&deps.seams);
    RelaySetup {
        logging: deps,
        logging_obj,
        request_body: IndexMap::from([("model".to_owned(), Json::Text("claude".to_owned()))]),
        failure_logging,
        registry,
        limits,
        boundaries,
    }
}

pub(super) struct Deferred {
    pub(super) calls: Arc<Mutex<Vec<usize>>>,
}

impl Deferred {
    pub(super) fn calls(&self) -> Vec<usize> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn arm_deferred(harness: &Harness) -> Deferred {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&calls);
    let callback: DeferredCallback = Box::new(move |args: Vec<LoggingFuture>| -> LoggingFuture {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(args.len());
        Box::pin(async move {
            for parked in args {
                parked.await?;
            }
            Ok(())
        })
    });
    harness.fields().deferred_callback = Some(callback);
    Deferred { calls }
}

pub(super) struct Hook {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

impl Hook {
    fn armed(harness: &Harness) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        harness.fields().detached_failure_hook = Some(Arc::new(RecordingHook {
            calls: Arc::clone(&calls),
        }));
        Self { calls }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct RecordingHook {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl DetachedFailureHook for RecordingHook {
    async fn on_detached_failure(
        &self,
        exception: &ReportedFailure,
    ) -> Result<(), ReportedFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exception.to_string());
        Ok(())
    }
}

pub(super) fn arm_hook(harness: &Harness) -> Hook {
    Hook::armed(harness)
}

pub(super) async fn settle(harness: &Harness, relay: ClientRelay<Recorder, ProviderError>) {
    drop(relay);
    harness.pump.wait().await;
}

pub(super) fn pump_of(harness: &Harness) -> Arc<TaskTracker> {
    Arc::clone(&harness.pump)
}

// The pump claims a drain slot on another worker thread, so the test waits for that claim
pub(super) async fn wait_for_drains(registry: &StreamRegistry, expected: usize) {
    for _ in 0_u16..1_000_u16 {
        if registry.sizes().detached_drains == expected {
            return;
        }
        yield_now().await;
    }
    assert_eq!(registry.sizes().detached_drains, expected);
}

pub(super) fn frame(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

pub(super) fn bytes(len: usize) -> Vec<u8> {
    vec![b'x'; len]
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Frame {
    Chunk(String),
    End,
    Error(String),
}

pub(super) async fn read(relay: &mut ClientRelay<Recorder, ProviderError>) -> Frame {
    match relay.next_client_chunk().await {
        Ok(Some(bytes)) => Frame::Chunk(String::from_utf8(bytes).expect("the fixture is UTF-8")),
        Ok(None) => Frame::End,
        Err(error) => Frame::Error(error.to_string()),
    }
}

pub(super) async fn release_and_read(
    harness: &Harness,
    relay: &mut ClientRelay<Recorder, ProviderError>,
    chunk: SseChunk,
) -> Frame {
    harness.script.release(chunk).await;
    read(relay).await
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
