#![cfg(test)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::state::OriginalFunction;
use betterlite_cache::replay::CachedDict;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::write_completion::tracker::CacheWriteTracker;
use betterlite_context::call_context::{CallContext, CallInputs};
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
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::api::messages_stream_logging::MessagesStreamLogging;
use crate::api::messages_stream_logging::StreamDeps;
use crate::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};
use crate::api::messages_stream_logging::slots::StreamSlots;
use crate::api::messages_stream_restamp::CACHED_STREAM_EVENTS_KEY;
use crate::api::messages_stream_restamp::cache_stream::{
    CacheWriteFailure, MessagesStreamCacheDeps, MessagesStreamCacheSettings,
    MessagesStreamCacheTarget,
};
use crate::api::messages_stream_restamp::replay::{CachedStreamLogging, ReplayLogging};

pub(super) const REQUESTED_MODEL: &str = "requested-model";
pub(super) const NOT_STORED: &str = "the stream write was not cached";

// One scripted transport read, then the end of the stream; a read failure ends the stream early
pub(super) enum Read {
    Chunk(Vec<u8>),
    Failure(&'static str),
}

pub(super) struct ScriptedStream {
    reads: VecDeque<Read>,
    buffered: Option<Json>,
    close_failure: Option<&'static str>,
}

impl ScriptedStream {
    pub(super) fn new(reads: Vec<Read>) -> Self {
        Self {
            reads: VecDeque::from(reads),
            buffered: None,
            close_failure: None,
        }
    }

    pub(super) fn with_buffered_output(mut self, buffered: Json) -> Self {
        self.buffered = Some(buffered);
        self
    }

    pub(super) fn with_close_failure(mut self, message: &'static str) -> Self {
        self.close_failure = Some(message);
        self
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct StreamFailure {
    pub(super) message: String,
}

#[async_trait]
impl MessagesProviderStream for ScriptedStream {
    type Error = StreamFailure;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StreamFailure> {
        match self.reads.pop_front() {
            Some(Read::Chunk(chunk)) => Ok(Some(chunk)),
            Some(Read::Failure(message)) => Err(StreamFailure {
                message: message.to_owned(),
            }),
            None => Ok(None),
        }
    }

    async fn aclose(&mut self) -> Option<Result<(), StreamFailure>> {
        Some(self.close_failure.map_or(Ok(()), |message| {
            Err(StreamFailure {
                message: message.to_owned(),
            })
        }))
    }

    fn buffered_provider_output(&self) -> Option<Json> {
        self.buffered.clone()
    }
}

// Records every stream the writer hands to the cache, and can fail the write the way a broken backend does
pub(super) struct RecordingCache {
    writes: Mutex<Vec<(Json, KeyObject)>>,
    failure: Option<String>,
}

impl RecordingCache {
    pub(super) fn succeeding() -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(Vec::new()),
            failure: None,
        })
    }

    pub(super) fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(Vec::new()),
            failure: Some(message.to_owned()),
        })
    }

    pub(super) fn writes(&self) -> Vec<(Json, KeyObject)> {
        mem::take(&mut *self.writes.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[async_trait]
impl MessagesStreamCacheTarget for RecordingCache {
    async fn add_cached_stream(
        &self,
        payload: Json,
        request_kwargs: KeyObject,
    ) -> Result<(), CacheWriteFailure> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((payload, request_kwargs));
        self.failure.as_ref().map_or(Ok(()), |message| {
            Err(CacheWriteFailure {
                message: message.clone(),
            })
        })
    }
}

#[derive(Default)]
struct LoggedSide {
    stamped: AtomicBool,
    billed: Mutex<Vec<Vec<Vec<u8>>>>,
}

// The logging seam a test drives: the stamp and the billed chunks it received
#[derive(Clone, Default)]
pub(super) struct RecordingLogging {
    side: Arc<LoggedSide>,
}

impl RecordingLogging {
    pub(super) fn stamped(&self) -> bool {
        self.side.stamped.load(Ordering::SeqCst)
    }

    pub(super) fn billed(&self) -> Vec<Vec<Vec<u8>>> {
        mem::take(
            &mut *self
                .side
                .billed
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
}

impl CachedStreamLogging for RecordingLogging {
    fn stamp_cache_hit(&self) {
        self.side.stamped.store(true, Ordering::SeqCst);
    }

    fn bill_cached_chunks(&self, collected_chunks: Vec<Vec<u8>>) {
        self.side
            .billed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(collected_chunks);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapturedEvent {
    pub(super) level: Level,
    pub(super) message: String,
}

#[derive(Clone, Default)]
pub(super) struct Capture(pub(super) Arc<Mutex<Vec<CapturedEvent>>>);

impl Capture {
    pub(super) fn events(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level > Level::WARN {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CapturedEvent {
                level,
                message: visitor.0,
            });
    }
}

pub(super) fn capture() -> (DefaultGuard, Capture) {
    let capture = Capture::default();
    let guard = set_default(registry().with(capture.clone()));
    (guard, capture)
}

pub(super) fn limits(max_items: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: ApiLimits::default().max_json_bytes,
        max_event_bytes: ApiLimits::default().max_event_bytes,
        max_items,
    }
}

pub(super) fn supported_call_types() -> Vec<CachingSupportedCallType> {
    vec![CachingSupportedCallType::AanthropicMessages]
}

pub(super) fn request_kwargs() -> KeyObject {
    KeyObject::from([
        ("model".to_owned(), KeyValue::String("claude-3".to_owned())),
        ("stream".to_owned(), KeyValue::Bool(true)),
    ])
}

pub(super) fn cache_settings(
    request_kwargs: KeyObject,
    preset: Option<&str>,
) -> MessagesStreamCacheSettings {
    MessagesStreamCacheSettings {
        original_function: OriginalFunction {
            name: "aanthropic_messages".to_owned(),
            parameters: Vec::new(),
        },
        supported_call_types: Some(supported_call_types()),
        request_kwargs,
        preset_cache_key: preset.map(str::to_owned),
    }
}

pub(super) fn cache_deps(
    cache: Option<Arc<RecordingCache>>,
    tracker: &Arc<CacheWriteTracker>,
    api_limits: ApiLimits,
) -> MessagesStreamCacheDeps {
    let target: Option<Arc<dyn MessagesStreamCacheTarget>> = cache.map(|recording| {
        let target: Arc<dyn MessagesStreamCacheTarget> = Arc::<RecordingCache>::clone(&recording);
        target
    });
    MessagesStreamCacheDeps {
        cache: target,
        tracker: Arc::clone(tracker),
        call: call_context(),
        limits: api_limits,
    }
}

pub(super) fn tracker() -> Arc<CacheWriteTracker> {
    Arc::new(CacheWriteTracker {
        tasks: TaskTracker::new(),
        cancellation: CancellationToken::new(),
    })
}

pub(super) fn call_context() -> CallContext<'static> {
    CallContext::start(CallInputs {
        now: || SystemTime::UNIX_EPOCH,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    })
}

// The producer's payload read back as the cache stores it: an object of event strings
pub(super) fn cached_dict(payload: &Json) -> CachedDict {
    let text = serde_json::to_string(payload).expect("the payload serializes");
    serde_json::from_str(&text).expect("the payload is a cached value")
}

pub(super) fn cached_events_dict(events: &[String]) -> CachedDict {
    let payload = Json::Object(IndexMap::from([(
        CACHED_STREAM_EVENTS_KEY.to_owned(),
        Json::List(
            events
                .iter()
                .map(|event| Json::Text(event.clone()))
                .collect(),
        ),
    )]));
    cached_dict(&payload)
}

pub(super) fn payload_events(payload: &Json) -> Vec<String> {
    let Json::Object(fields) = payload else {
        panic!("the payload is an object");
    };
    let Some(Json::List(events)) = fields.get(CACHED_STREAM_EVENTS_KEY) else {
        panic!("the payload carries the cached events key");
    };
    events
        .iter()
        .map(|event| match event {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                panic!("a cached event is text");
            }
        })
        .collect()
}

fn base_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        base_time()
    }
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
            model: "claude-3".to_owned(),
            call_type: "aanthropic_messages".to_owned(),
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

// Records every seam call B2436's stream logging makes; the future it hands back only counts its run
pub(super) struct SeamRecorder {
    pub(super) routed: Mutex<Vec<RouteStreamingLogging>>,
    pub(super) processed: Mutex<Vec<ChunkProcessorCall<String>>>,
}

impl StreamSeams for SeamRecorder {
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
        Ok(Box::pin(async { Ok(()) }))
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

// The production replay seam over B2436's stream logging, with the worker and slots kept alive beside it
pub(super) struct ReplayHarness {
    pub(super) seams: Arc<SeamRecorder>,
    pub(super) logging_obj: Arc<StreamLoggingObject>,
    pub(super) replay: ReplayLogging<SeamRecorder>,
}

impl ReplayHarness {
    pub(super) fn new() -> Self {
        let seams = Arc::new(SeamRecorder {
            routed: Mutex::new(Vec::new()),
            processed: Mutex::new(Vec::new()),
        });
        let worker =
            Arc::new(LoggingWorker::start(WorkerSettings::default()).expect("the worker starts"));
        let slots = Arc::new(StreamSlots::default());
        let success_handler = Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        });
        let logging_obj = logging_object();
        let deps = StreamDeps {
            seams: Arc::clone(&seams),
            worker,
            slots,
            clock: Arc::new(FixedClock),
            success_handler,
        };
        let request_body =
            IndexMap::from([("model".to_owned(), Json::Text("claude-3".to_owned()))]);
        let stream = MessagesStreamLogging::new(deps, Arc::clone(&logging_obj), request_body);
        Self {
            seams,
            replay: ReplayLogging {
                logging_obj: Arc::clone(&logging_obj),
                stream,
            },
            logging_obj,
        }
    }
}

pub(super) fn cache_hit(logging_obj: &StreamLoggingObject) -> Option<Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.get("cache_hit").cloned())
        .flatten()
}
