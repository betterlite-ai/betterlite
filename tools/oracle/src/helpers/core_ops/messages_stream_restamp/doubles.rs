use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::state::OriginalFunction;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::api::messages_stream_restamp::MessagesStreamChunk;
use betterlite_core::api::messages_stream_restamp::cache_stream::{
    CacheWriteFailure, MessagesStreamCacheSettings, MessagesStreamCacheTarget,
};
use betterlite_core::api::messages_stream_restamp::replay::CachedStreamLogging;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};
use uuid::Uuid;

const REQUEST_MODEL: &str = "claude-3";
const ORIGINAL_FUNCTION: &str = "aanthropic_messages";

pub(super) fn chunk_value(chunk: &MessagesStreamChunk) -> Value {
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

pub(super) fn writes_value(writes: &[(Value, KeyObject)]) -> Value {
    let entries: Vec<Value> = writes
        .iter()
        .map(|(payload, kwargs)| json!({ "payload": payload, "kwargs": key_object(kwargs) }))
        .collect();
    Value::Array(entries)
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

pub(super) fn cache_settings(preset: Option<String>) -> MessagesStreamCacheSettings {
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

pub(super) fn call_context() -> CallContext<'static> {
    CallContext::start(CallInputs {
        now: || SystemTime::UNIX_EPOCH,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    })
}

pub(super) struct ScriptedStream {
    reads: VecDeque<Vec<u8>>,
}

impl ScriptedStream {
    pub(super) fn new(reads: Vec<Vec<u8>>) -> Self {
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

pub(super) struct RecordingCache {
    pub(super) writes: Mutex<Vec<(Value, KeyObject)>>,
    failure: Option<String>,
}

impl RecordingCache {
    pub(super) const fn new(failure: Option<String>) -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            failure,
        }
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
        self.failure.as_ref().map_or(Ok(()), |message| {
            Err(CacheWriteFailure {
                message: message.clone(),
            })
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingLogging {
    pub(super) stamped: Arc<AtomicBool>,
}

impl CachedStreamLogging for RecordingLogging {
    fn stamp_cache_hit(&self) {
        self.stamped.store(true, Ordering::SeqCst);
    }

    fn bill_cached_chunks(&self, _collected_chunks: Vec<Vec<u8>>) {}
}
