use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::logging_streams::assembly::StreamAssemblyError;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use indexmap::IndexMap;
use serde_json::Number;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::backend::memory::MemoryCacheOps;
use crate::backend::memory::local::LocalMemoryOps;
use crate::backend::{CacheResult, Ttl};
use crate::cache_key::settings::CacheKeySettings;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::dual_cache::{DualCache, DualCacheSettings};
use crate::embedding::response::{EmbeddingRecord, EmbeddingResponse};
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use crate::envelope::lookup::CacheLookup;
use crate::handler::state::{CachingHandler, HandlerDependencies, OriginalFunction};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::CachingSupportedCallType;
use crate::types::embedding::EmbeddingPayload;
use crate::types::semantic::SemanticCacheScope;
use crate::write_completion::result::CacheableResult;
use crate::write_completion::store::{CacheContext, WriteContext};
use crate::write_completion::stream::{CacheStreamAssembler, StreamAssembly};
use crate::write_completion::tracker::CacheWriteTracker;

pub(crate) struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

pub(crate) struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

pub(crate) type TestCache = ResponseCache<SilentLogger, SilentConsole>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Sink {
    Direct,
    Dual,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordedWrite {
    pub(crate) sink: Sink,
    pub(crate) key: String,
    pub(crate) response: CacheValue,
}

#[derive(Default)]
pub(crate) struct Recorder {
    writes: Mutex<Vec<RecordedWrite>>,
}

impl Recorder {
    pub(crate) fn record(&self, sink: Sink, key: &str, value: &CacheValue) {
        let response = match value {
            CacheValue::Object(fields) => {
                fields.get("response").cloned().unwrap_or(CacheValue::Null)
            }
            CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Integer(_)
            | CacheValue::Float(_)
            | CacheValue::Text(_)
            | CacheValue::Array(_)
            | CacheValue::Bytes(_)
            | CacheValue::Credential(_) => CacheValue::Null,
        };
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.push(RecordedWrite {
            sink,
            key: key.to_owned(),
            response,
        });
    }

    pub(crate) fn writes(&self) -> Vec<RecordedWrite> {
        let writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.clone()
    }
}

pub(crate) struct RecordingBackend {
    pub(crate) recorder: Arc<Recorder>,
}

#[async_trait]
impl EnvelopeBackend for RecordingBackend {
    fn get_cache(&self, _key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Direct, key, &value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        _key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Direct, key, &value);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Direct, key, &value);
        Ok(())
    }
}

pub(crate) struct RecordingMemory {
    pub(crate) recorder: Arc<Recorder>,
    pub(crate) stored: Mutex<BTreeMap<String, CacheValue>>,
}

impl MemoryCacheOps for RecordingMemory {
    fn default_ttl(&self) -> i64 {
        600
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(stored.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Dual, key, &value);
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.insert(key.to_owned(), value);
        Ok(())
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.remove(key);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.clear();
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }

    fn increment_cache(&self, _key: &str, value: f64, _ttl: Ttl) -> CacheResult<f64> {
        Ok(value)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Ok(Vec::new())
    }
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
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

pub(crate) fn capture_warnings() -> (impl tracing::Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .with_writer(move || Capture(Arc::clone(&sink)))
        .finish();
    (subscriber, bytes)
}

pub(crate) fn captured(bytes: &Mutex<Vec<u8>>) -> String {
    let log = bytes.lock().unwrap_or_else(PoisonError::into_inner).clone();
    String::from_utf8(log).expect("the log is utf-8")
}

pub(crate) fn tracker() -> CacheWriteTracker {
    CacheWriteTracker {
        tasks: TaskTracker::new(),
        cancellation: CancellationToken::new(),
    }
}

pub(crate) fn hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    }
}

pub(crate) fn cache(recorder: &Arc<Recorder>, cache_type: LiteLlmCacheType) -> Arc<TestCache> {
    let clock: Clock = Arc::new(|| 0.0_f64);
    Arc::new(ResponseCache {
        backend: Arc::new(RecordingBackend {
            recorder: Arc::clone(recorder),
        }),
        key_settings: CacheKeySettings {
            cache_type,
            mode: CacheMode::DefaultOn,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: EnvelopeSettings {
            ttl: None,
            redis_flush_size: None,
        },
        hooks: hooks(),
        clock,
    })
}

pub(crate) fn call() -> CallContext<'static> {
    CallContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    })
}

pub(crate) fn write_context<'call>(
    recorder: &Arc<Recorder>,
    cache_type: LiteLlmCacheType,
    supported: Option<Vec<CachingSupportedCallType>>,
    call: &'call CallContext<'call>,
) -> WriteContext<'call, SilentLogger, SilentConsole> {
    WriteContext {
        cache: CacheContext {
            cache: Some(cache(recorder, cache_type)),
            supported_call_types: supported,
        },
        tracker: Arc::new(tracker()),
        call,
    }
}

pub(crate) fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1000,
        redis_delete_chunk_size: 1000,
    }
}

pub(crate) fn recording_memory(recorder: &Arc<Recorder>) -> Arc<RecordingMemory> {
    Arc::new(RecordingMemory {
        recorder: Arc::clone(recorder),
        stored: Mutex::new(BTreeMap::new()),
    })
}

pub(crate) fn recording_dual(recorder: &Arc<Recorder>) -> DualCache<RecordingMemory> {
    DualCache::new(recording_memory(recorder), None, dual_settings())
}

pub(crate) fn memory() -> Arc<LocalMemoryOps> {
    let clock: Clock = Arc::new(|| 0.0_f64);
    let operations = LocalMemoryOps::new(1024, clock).expect("the memory tier builds");
    Arc::new(operations)
}

pub(crate) fn handler(kwargs: KeyObject) -> CachingHandler {
    let dependencies = HandlerDependencies {
        original_function: original("acompletion", &["model", "messages"]),
        start_time: 0.0,
        redis_cache: None,
        in_memory_cache: memory(),
        dual_settings: dual_settings(),
    };
    CachingHandler::new(dependencies, kwargs)
}

pub(crate) fn original(name: &str, parameters: &[&str]) -> OriginalFunction {
    OriginalFunction {
        name: name.to_owned(),
        parameters: parameters.iter().copied().map(str::to_owned).collect(),
    }
}

pub(crate) fn kwargs(entries: &[(&str, KeyValue)]) -> KeyObject {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn text(value: &str) -> KeyValue {
    KeyValue::String(value.to_owned())
}

pub(crate) fn object(entries: &[(&str, KeyValue)]) -> KeyValue {
    KeyValue::Object(kwargs(entries))
}

pub(crate) fn chat_kwargs() -> KeyObject {
    kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
    ])
}

pub(crate) fn embedding_kwargs(inputs: &[&str]) -> KeyObject {
    kwargs(&[
        ("model", text("text-embedding-3-small")),
        (
            "input",
            KeyValue::Array(inputs.iter().map(|value| text(*value)).collect()),
        ),
    ])
}

pub(crate) fn chat_json() -> Json {
    Json::Object(IndexMap::from([
        ("model".to_owned(), Json::Text("gpt-4".to_owned())),
        ("choices".to_owned(), Json::List(Vec::new())),
    ]))
}

pub(crate) fn chat_result() -> CacheableResult {
    CacheableResult::Chat(chat_json())
}

pub(crate) fn other_json() -> Json {
    Json::Object(IndexMap::from([(
        "custom".to_owned(),
        Json::Text("value".to_owned()),
    )]))
}

pub(crate) fn text_completion_json() -> Json {
    Json::Object(IndexMap::from([
        (
            "object".to_owned(),
            Json::Text("text_completion".to_owned()),
        ),
        ("choices".to_owned(), Json::List(Vec::new())),
    ]))
}

pub(crate) fn other_result() -> CacheableResult {
    CacheableResult::Other(other_json())
}

pub(crate) fn embedding_response(count: usize) -> EmbeddingResponse {
    let records: Vec<EmbeddingRecord> = (0..count)
        .map(|index| EmbeddingRecord {
            embedding: Some(EmbeddingPayload::Vector(vec![
                Number::from_f64(0.5).expect("the fixture float is finite"),
            ])),
            index: Some(i64::try_from(index).expect("the fixture index fits")),
            object: Some("embedding".to_owned()),
        })
        .collect();
    EmbeddingResponse {
        model: Some("text-embedding-3-small".to_owned()),
        data: records,
        usage: None,
    }
}

pub(crate) fn embedding_result(count: usize) -> CacheableResult {
    CacheableResult::Embedding(embedding_response(count))
}

pub(crate) fn chunk(content: &str, finish_reason: Option<&str>) -> SuccessResult {
    let finish = match finish_reason {
        Some(reason) => Json::Text(reason.to_owned()),
        None => Json::Null,
    };
    let delta = Json::Object(IndexMap::from([(
        "content".to_owned(),
        Json::Text(content.to_owned()),
    )]));
    let choice = Json::Object(IndexMap::from([
        ("delta".to_owned(), delta),
        ("finish_reason".to_owned(), finish),
    ]));
    SuccessResult::Chunk(Json::Object(IndexMap::from([(
        "choices".to_owned(),
        Json::List(vec![choice]),
    )])))
}

#[derive(Clone)]
pub(crate) struct AssemblyCall {
    pub(crate) start_time: f64,
    pub(crate) end_time: f64,
    pub(crate) is_async: bool,
    pub(crate) chunks: Vec<CacheValue>,
}

pub(crate) struct FakeAssembler {
    pub(crate) response: Mutex<Option<Json>>,
    pub(crate) failure: bool,
    pub(crate) calls: Mutex<Vec<AssemblyCall>>,
}

impl FakeAssembler {
    pub(crate) fn calls(&self) -> Vec<AssemblyCall> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl CacheStreamAssembler for FakeAssembler {
    fn assemble(&self, request: &StreamAssembly<'_>) -> Result<Option<Json>, StreamAssemblyError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(AssemblyCall {
            start_time: request.start_time,
            end_time: request.end_time,
            is_async: request.is_async,
            chunks: request.chunks.to_vec(),
        });
        drop(calls);
        if self.failure {
            return Err(StreamAssemblyError::Builder(Box::new(io::Error::other(
                "assembly failed",
            ))));
        }
        let response = self.response.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(response.clone())
    }
}
