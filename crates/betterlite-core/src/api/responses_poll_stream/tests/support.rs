use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::poll_fn;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::vec::IntoIter;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use tokio::sync::oneshot::Receiver;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use crate::api::responses_poll_state::get_cache_key;
use crate::api::responses_poll_state::store::ResponsePollingStore;
use crate::api::responses_poll_stream::clock::PollClock;
use crate::api::responses_poll_stream::ports::{
    ClientInbound, PollChunkSource, PollProcessCall, PollProcessed, PollProcessorError,
    PollRequest, PollRequestProcessor, PollScope,
};
use crate::api::responses_poll_stream::worker::{BackgroundStreaming, PollStreamError};

pub(super) const POLLING_ID: &str = "litellm_poll_b936";

pub(super) const REQUEST_BODY: &str =
    r#"{"background":true,"input":"Hello","model":"b936-test-model","stream":false}"#;
const SEED: &str = r#"{"created_at":100,"id":"litellm_poll_b936","object":"response","output":[],"status":"queued"}"#;

#[derive(Debug)]
pub(super) enum Answer {
    Streaming(Vec<AnswerStep>),
    NonStreaming,
    Failure(String),
}

#[derive(Debug)]
pub(super) enum AnswerStep {
    Chunk(Vec<u8>),
    Wait(Receiver<()>),
    Fail(String),
}

#[derive(Debug, Clone)]
pub(super) struct CapturedCall {
    pub(super) route_type: &'static str,
    pub(super) model: Option<String>,
    pub(super) skip_pre_call_logic: bool,
    pub(super) data: IndexMap<String, Json>,
    pub(super) scope: PollScope,
    pub(super) detached_receive_pending: bool,
}

pub(super) struct ScriptedProcessor {
    answer: Mutex<Option<Answer>>,
    captured: Mutex<Option<CapturedCall>>,
    pub(super) calls: AtomicUsize,
}

struct ScriptedChunks {
    steps: IntoIter<AnswerStep>,
}

#[async_trait]
impl PollChunkSource for ScriptedChunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, PollProcessorError> {
        for step in &mut self.steps {
            match step {
                AnswerStep::Chunk(bytes) => return Ok(Some(bytes)),
                AnswerStep::Wait(receiver) => {
                    receiver.await.map_err(|_error| PollProcessorError {
                        message: String::from("the release signal never arrived"),
                    })?;
                }
                AnswerStep::Fail(message) => return Err(PollProcessorError { message }),
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl PollRequestProcessor for ScriptedProcessor {
    async fn process(
        &self,
        mut call: PollProcessCall<'_>,
    ) -> Result<PollProcessed, PollProcessorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let scope = call.request.scope.clone();
        let mut receive = Box::pin(call.request.receive());
        let first = poll_fn(|context| Poll::Ready(receive.as_mut().poll(context))).await;
        let captured = CapturedCall {
            route_type: call.route_type,
            model: call.model.clone(),
            skip_pre_call_logic: call.skip_pre_call_logic,
            data: call.data.clone(),
            scope,
            detached_receive_pending: first.is_pending(),
        };
        *self.captured.lock().unwrap_or_else(PoisonError::into_inner) = Some(captured);
        let answer = self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match answer {
            Some(Answer::Streaming(steps)) => {
                Ok(PollProcessed::Streaming(Box::new(ScriptedChunks {
                    steps: steps.into_iter(),
                })))
            }
            Some(Answer::NonStreaming) => Ok(PollProcessed::NonStreaming),
            Some(Answer::Failure(message)) => Err(PollProcessorError { message }),
            None => Err(PollProcessorError {
                message: String::from("the scripted answer was already taken"),
            }),
        }
    }
}

impl ScriptedProcessor {
    pub(super) fn captured(&self) -> CapturedCall {
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("the processor captured the provider call")
    }
}

pub(super) struct ScriptedClock {
    readings: Mutex<VecDeque<u64>>,
    pub(super) reads: AtomicUsize,
}

impl PollClock for ScriptedClock {
    fn now_millis(&self) -> u64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut readings = self.readings.lock().unwrap_or_else(PoisonError::into_inner);
        if readings.len() > 1 {
            readings.pop_front().unwrap_or(0)
        } else {
            readings.front().copied().unwrap_or(0)
        }
    }
}

pub(super) struct RecordingCache {
    entries: Mutex<BTreeMap<String, String>>,
    written: Mutex<Vec<String>>,
    pub(super) attempted: AtomicUsize,
    failing_at: AtomicUsize,
    pub(super) reads: AtomicUsize,
}

impl Default for RecordingCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            written: Mutex::new(Vec::new()),
            attempted: AtomicUsize::new(0),
            failing_at: AtomicUsize::new(usize::MAX),
            reads: AtomicUsize::new(0),
        }
    }
}

impl RecordingCache {
    pub(super) fn seeded() -> Arc<Self> {
        let cache = Arc::new(Self::default());
        cache.seed(SEED);
        cache
    }

    fn entries(&self) -> MutexGuard<'_, BTreeMap<String, String>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn written(&self) -> MutexGuard<'_, Vec<String>> {
        self.written.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn seed(&self, source: &str) {
        self.entries()
            .insert(get_cache_key(POLLING_ID), source.to_owned());
    }

    pub(super) fn last_write_bytes(&self) -> Vec<u8> {
        self.written()
            .last()
            .map(|text| text.clone().into_bytes())
            .expect("the worker wrote the record")
    }

    pub(super) fn writes(&self) -> Vec<Json> {
        self.written().iter().map(|text| json(text)).collect()
    }

    pub(super) fn fail_write_at(&self, position: usize) {
        self.failing_at.store(position, Ordering::SeqCst);
    }
}

#[async_trait]
impl PollingCache for RecordingCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.entries().get(key).cloned().map(String::into_bytes))
    }

    async fn write(&self, key: &str, value: &str, _ttl: i64) -> Result<(), PollingCacheError> {
        let position = self.attempted.fetch_add(1, Ordering::SeqCst);
        if position == self.failing_at.load(Ordering::SeqCst) {
            self.failing_at.store(usize::MAX, Ordering::SeqCst);
            return Err(PollingCacheError::Write(String::from(
                "scripted write failure",
            )));
        }
        self.written().push(value.to_owned());
        self.entries().insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        Ok(i64::from(self.entries().remove(key).is_some()))
    }
}

#[derive(Default)]
pub(super) struct RecordingInbound {
    pub(super) received: AtomicUsize,
}

struct SharedInbound(Arc<RecordingInbound>);

#[async_trait]
impl ClientInbound for SharedInbound {
    async fn receive(&mut self) {
        self.0.received.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) struct Scenario {
    pub(super) cache: Arc<RecordingCache>,
    pub(super) clock: Arc<ScriptedClock>,
    pub(super) processor: Arc<ScriptedProcessor>,
    pub(super) inbound: Arc<RecordingInbound>,
    pub(super) store: ResponsePollingStore,
    request: Option<PollRequest>,
}

pub(super) fn scenario(readings: &[u64], answer: Answer) -> Scenario {
    let cache = RecordingCache::seeded();
    let store = store_for(&cache, ApiLimits::default());
    let inbound = Arc::new(RecordingInbound::default());
    let request = PollRequest {
        scope: PollScope {
            method: String::from("POST"),
            path: String::from("/v1/responses"),
            headers: vec![
                (
                    String::from("authorization"),
                    String::from("Bearer sk-b936"),
                ),
                (
                    String::from("content-type"),
                    String::from("application/json"),
                ),
            ],
            body: REQUEST_BODY.as_bytes().to_vec(),
        },
        inbound: Box::new(SharedInbound(Arc::clone(&inbound))),
    };
    Scenario {
        cache,
        clock: Arc::new(clock(readings)),
        processor: Arc::new(ScriptedProcessor {
            answer: Mutex::new(Some(answer)),
            captured: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }),
        inbound,
        store,
        request: Some(request),
    }
}

pub(super) fn clock(readings: &[u64]) -> ScriptedClock {
    ScriptedClock {
        readings: Mutex::new(readings.iter().copied().collect()),
        reads: AtomicUsize::new(0),
    }
}

pub(super) fn store_for(cache: &Arc<RecordingCache>, limits: ApiLimits) -> ResponsePollingStore {
    let handle: Option<Arc<dyn PollingCache>> = Some(Arc::<RecordingCache>::clone(cache));
    ResponsePollingStore {
        cache: handle,
        ttl: 3600,
        limits,
    }
}

impl Scenario {
    pub(super) async fn run(&mut self) -> Result<(), PollStreamError> {
        let Some(request) = self.request.take() else {
            panic!("a scenario runs once");
        };
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds its patterns");
        BackgroundStreaming {
            polling_id: POLLING_ID,
            data: request_data(),
            request,
            store: &self.store,
            processor: self.processor.as_ref(),
            clock: self.clock.as_ref(),
            redactor: &redactor,
        }
        .run()
        .await
    }

    // B1087 resolves one ApiLimits for the store's reads and the worker's writes, so a scenario scripts that copy
    pub(super) fn with_limits(mut self, limits: ApiLimits) -> Self {
        self.store.limits = limits;
        self
    }
}

pub(super) fn request_data() -> IndexMap<String, Json> {
    let Json::Object(data) = json(REQUEST_BODY) else {
        panic!("the admitted request body is an object");
    };
    data
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

pub(super) fn seeded_response() -> Json {
    json(SEED)
}

pub(super) fn frames(events: &[&str]) -> Vec<Vec<u8>> {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n").into_bytes())
        .collect()
}

pub(super) fn field(record: &Json, name: &str) -> Json {
    let Json::Object(fields) = record else {
        panic!("the record is an object");
    };
    fields.get(name).cloned().unwrap_or(Json::Null)
}

// a record the store wrote since owners existed keeps the response behind its envelope
pub(super) fn response(record: &Json) -> Json {
    match field(record, "response") {
        value @ Json::Object(_) => value,
        Json::Null => record.clone(),
        other => panic!("response is not an object: {other:?}"),
    }
}

pub(super) fn text(record: &Json, name: &str) -> String {
    match field(record, name) {
        Json::Text(value) => value,
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            panic!("{name} is not text: {other:?}")
        }
    }
}

pub(super) fn last_write(cache: &RecordingCache) -> Json {
    let writes = cache.writes();
    let Some(record) = writes.into_iter().next_back() else {
        panic!("the worker wrote the record");
    };
    record
}

pub(super) fn first_text(record: &Json) -> String {
    let Json::List(items) = field(record, "output") else {
        panic!("the output is a list");
    };
    let Some(first) = items.first() else {
        panic!("the output has an item");
    };
    let Json::List(parts) = field(first, "content") else {
        panic!("the item has content parts");
    };
    let Some(part) = parts.first() else {
        panic!("the item has a part");
    };
    text(part, "text")
}

#[derive(Clone, Default)]
pub(super) struct Logs {
    lines: Arc<Mutex<Vec<LogLine>>>,
}

#[derive(Debug, Clone)]
struct LogLine {
    level: String,
    message: String,
}

#[derive(Default)]
struct Captured(String);

impl Visit for Captured {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = event.metadata().level().as_str().to_owned();
        let mut captured = Captured::default();
        event.record(&mut captured);
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogLine {
                level,
                message: captured.0,
            });
    }
}

impl Logs {
    pub(super) fn messages(&self, level: &str) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|line| line.level == level)
            .map(|line| line.message.clone())
            .collect()
    }
}

pub(super) fn capture_logs() -> (DefaultGuard, Logs) {
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    (set_default(subscriber), logs)
}
