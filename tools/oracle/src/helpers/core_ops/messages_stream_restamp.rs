use std::collections::VecDeque;
use std::convert::Infallible;
use std::mem::take;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::state::OriginalFunction;
use betterlite_cache::replay::CachedDict;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::write_completion::tracker::CacheWriteTracker;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::api::messages_stream_restamp::cache_stream::{
    CacheWriteFailure, MessagesStreamCacheDeps, MessagesStreamCacheSettings,
    MessagesStreamCacheTarget, MessagesStreamCacheWriter,
};
use betterlite_core::api::messages_stream_restamp::replay::{
    CachedMessagesResult, CachedStreamLogging, convert_cached_messages_result,
};
use betterlite_core::api::messages_stream_restamp::{
    CACHED_STREAM_EVENTS_KEY, MessagesStreamChunk, MessagesStreamRestampInput,
    restamp_messages_stream,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;
use uuid::Uuid;

mod capture;

const REQUEST_MODEL: &str = "claude-3";
const ORIGINAL_FUNCTION: &str = "aanthropic_messages";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    restamp: Option<RestampInput>,
    cache_stream: Option<CacheStreamInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestampInput {
    requested_model: String,
    chunks: Vec<ChunkSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChunkSpec {
    Event { value: Value },
    Bytes { value: String },
    Text { value: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheStreamInput {
    reads: Vec<String>,
    preset_cache_key: Option<String>,
    fail: Option<String>,
    replay: bool,
}

struct Recorded {
    yielded: Vec<String>,
    writes: Vec<(Value, KeyObject)>,
    replayed: Option<Vec<String>>,
    cache_hit: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({
            "restamp": input.restamp.as_ref().map(restamp_answer).transpose()?,
            "cache_stream": input.cache_stream.as_ref().map(cache_stream_answer).transpose()?,
        }))
    }
}

fn restamp_answer(input: &RestampInput) -> Result<Value, Box<HelperError>> {
    let chunks = input
        .chunks
        .iter()
        .map(chunk_from_spec)
        .collect::<Result<Vec<_>, _>>()?;
    let result = restamp_messages_stream(&MessagesStreamRestampInput {
        requested_model: input.requested_model.clone(),
        chunks,
    });
    Ok(json!({
        "chunks": result.chunks.iter().map(chunk_value).collect::<Vec<_>>(),
        "flushed": chunk_value(&result.flushed),
    }))
}

fn chunk_from_spec(spec: &ChunkSpec) -> Result<MessagesStreamChunk, Box<HelperError>> {
    match spec {
        ChunkSpec::Event { value } => serde_json::from_value(value.clone())
            .or_value_error()
            .map(MessagesStreamChunk::Event),
        ChunkSpec::Bytes { value } => BASE64
            .decode(value)
            .or_value_error()
            .map(MessagesStreamChunk::Bytes),
        ChunkSpec::Text { value } => Ok(MessagesStreamChunk::Text(value.clone())),
    }
}

fn chunk_value(chunk: &MessagesStreamChunk) -> Value {
    match chunk {
        MessagesStreamChunk::Event(event) => json!({
            "kind": "event",
            "value": serde_json::to_value(event).unwrap_or(Value::Null),
        }),
        MessagesStreamChunk::Bytes(bytes) => json!({
            "kind": "bytes",
            "value": BASE64.encode(bytes),
        }),
        MessagesStreamChunk::Text(text) => json!({"kind": "text", "value": text}),
    }
}

fn cache_stream_answer(input: &CacheStreamInput) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let cache = Arc::new(RecordingCache::new(input.fail.clone()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let guard = subscriber::set_default(
        tracing_subscriber::registry().with(capture::Capture(Arc::clone(&events))),
    );
    let recorded = runtime.block_on(write_stream(input, Arc::clone(&cache)));
    drop(guard);
    let recorded = recorded?;
    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({
        "yielded": recorded.yielded,
        "writes": writes_value(&recorded.writes),
        "events": captured,
        "replayed": recorded.replayed,
        "cache_hit": recorded.cache_hit,
    }))
}

async fn write_stream(
    input: &CacheStreamInput,
    cache: Arc<RecordingCache>,
) -> Result<Recorded, Box<HelperError>> {
    let tracker = Arc::new(CacheWriteTracker {
        tasks: TaskTracker::new(),
        cancellation: CancellationToken::new(),
    });
    let target: Arc<dyn MessagesStreamCacheTarget> = cache.clone();
    let reads = input
        .reads
        .iter()
        .map(|read| read.as_bytes().to_vec())
        .collect();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(reads),
        cache_settings(input.preset_cache_key.clone()),
        MessagesStreamCacheDeps {
            cache: Some(target),
            tracker: Arc::clone(&tracker),
            call: call_context(),
            limits: ApiLimits::default(),
        },
    );
    let yielded = drain(&mut writer).await?;
    tracker.wait_idle().await;
    let writes = cache.writes();
    let (replayed, cache_hit) = replay_written(input.replay, &writes).await?;
    Ok(Recorded {
        yielded: texts(yielded)?,
        writes,
        replayed,
        cache_hit,
    })
}

async fn drain(
    writer: &mut MessagesStreamCacheWriter<ScriptedStream>,
) -> Result<Vec<Vec<u8>>, Box<HelperError>> {
    let mut yielded = Vec::new();
    while let Some(chunk) = writer.next_chunk().await.or_value_error()? {
        yielded.push(chunk);
    }
    Ok(yielded)
}

async fn replay_written(
    replay: bool,
    writes: &[(Value, KeyObject)],
) -> Result<(Option<Vec<String>>, bool), Box<HelperError>> {
    if !replay {
        return Ok((None, false));
    }
    let Some((payload, _)) = writes.first() else {
        return Ok((None, false));
    };
    let cached: CachedDict = serde_json::from_value(payload.clone()).or_value_error()?;
    let logging = RecordingLogging::default();
    let converted = convert_cached_messages_result(cached, logging.clone()).or_value_error()?;
    let CachedMessagesResult::Stream(mut iterator) = converted else {
        return Ok((None, logging.stamped()));
    };
    let mut events = Vec::new();
    while let Some(chunk) = iterator.next_chunk().await.or_value_error()? {
        events.push(text_of(chunk)?);
    }
    Ok((Some(events), logging.stamped()))
}

fn text_of(bytes: Vec<u8>) -> Result<String, Box<HelperError>> {
    String::from_utf8(bytes).or_value_error()
}

fn texts(chunks: Vec<Vec<u8>>) -> Result<Vec<String>, Box<HelperError>> {
    chunks.into_iter().map(text_of).collect()
}

fn writes_value(writes: &[(Value, KeyObject)]) -> Value {
    Value::Array(
        writes
            .iter()
            .map(|(payload, kwargs)| {
                json!({
                    "events": payload.get(CACHED_STREAM_EVENTS_KEY).cloned().unwrap_or(Value::Null),
                    "kwargs": key_object(kwargs),
                })
            })
            .collect(),
    )
}

fn key_object(kwargs: &KeyObject) -> Value {
    let mut fields = serde_json::Map::new();
    for (name, value) in kwargs {
        fields.insert(name.clone(), key_value(value));
    }
    Value::Object(fields)
}

fn key_value(value: &KeyValue) -> Value {
    match value {
        KeyValue::Null => Value::Null,
        KeyValue::Bool(flag) => Value::Bool(*flag),
        KeyValue::Number(number) => Value::Number(number.clone()),
        KeyValue::String(text) => Value::String(text.clone()),
        KeyValue::Array(items) => Value::Array(items.iter().map(key_value).collect()),
        KeyValue::Object(fields) => key_object(fields),
    }
}

fn cache_settings(preset: Option<String>) -> MessagesStreamCacheSettings {
    MessagesStreamCacheSettings {
        original_function: OriginalFunction {
            name: ORIGINAL_FUNCTION.to_owned(),
            parameters: Vec::new(),
        },
        supported_call_types: Some(vec![CachingSupportedCallType::AanthropicMessages]),
        request_kwargs: KeyObject::from([
            (
                "model".to_owned(),
                KeyValue::String(REQUEST_MODEL.to_owned()),
            ),
            ("stream".to_owned(), KeyValue::Bool(true)),
        ]),
        preset_cache_key: preset,
    }
}

fn call_context() -> CallContext<'static> {
    CallContext::start(CallInputs {
        now: || SystemTime::UNIX_EPOCH,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    })
}

struct ScriptedStream {
    reads: VecDeque<Vec<u8>>,
}

impl ScriptedStream {
    fn new(reads: Vec<Vec<u8>>) -> Self {
        Self {
            reads: VecDeque::from(reads),
        }
    }
}

#[async_trait]
impl MessagesProviderStream for ScriptedStream {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        Ok(self.reads.pop_front())
    }
}

struct RecordingCache {
    writes: Mutex<Vec<(Value, KeyObject)>>,
    failure: Option<String>,
}

impl RecordingCache {
    fn new(failure: Option<String>) -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            failure,
        }
    }

    fn writes(&self) -> Vec<(Value, KeyObject)> {
        take(&mut *self.writes.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[async_trait]
impl MessagesStreamCacheTarget for RecordingCache {
    async fn add_cached_stream(
        &self,
        payload: Json,
        request_kwargs: KeyObject,
    ) -> Result<(), CacheWriteFailure> {
        let recorded = serde_json::to_value(&payload).unwrap_or(Value::Null);
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((recorded, request_kwargs));
        match &self.failure {
            Some(message) => Err(CacheWriteFailure {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Default)]
struct RecordingLogging {
    stamped: Arc<AtomicBool>,
}

impl RecordingLogging {
    fn stamped(&self) -> bool {
        self.stamped.load(Ordering::SeqCst)
    }
}

impl CachedStreamLogging for RecordingLogging {
    fn stamp_cache_hit(&self) {
        self.stamped.store(true, Ordering::SeqCst);
    }

    fn bill_cached_chunks(&self, _collected_chunks: Vec<Vec<u8>>) {}
}
