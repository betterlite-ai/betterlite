#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::{Extra, Json};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde_json::Number;

use crate::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use crate::cache_key::request::{CacheKeyRequest, KeyParam};
use crate::cache_key::settings::CacheKeySettings;
use crate::cache_key::value::KeyValue;
use crate::codec::CacheCodec;
use crate::embedding::embedding_cache::EmbeddingCacheEntry;
use crate::embedding::embedding_cache::pipeline::{NativeCacheRequest, NativeEmbeddingStore};
use crate::embedding::response::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use crate::envelope::backend::StoredBackend;
use crate::envelope::facade::{EnvelopeSettings, ResponseCache};
use crate::envelope::request::EnvelopeRequest;
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::control::DynamicCacheControl;
use crate::types::embedding::{CachedEmbedding, EmbeddingPayload};
use crate::types::semantic::SemanticCacheScope;

const NOW: f64 = 1_700_000_000.0;

#[derive(Default)]
pub(super) struct StoringBackend {
    values: Mutex<BTreeMap<String, CacheValue>>,
}

#[async_trait]
impl CacheBackend for StoringBackend {
    fn default_ttl(&self) -> i64 {
        60
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        Ok(())
    }

    async fn async_get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.get_cache(key)
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.set_cache(key, value, None)
    }
}

type Stored = (String, CacheValue, Ttl);
type PipelineWrite = (Vec<(String, CacheValue)>, Ttl);

#[derive(Default)]
pub(super) struct RecordingLogger {
    records: Mutex<Vec<(String, String)>>,
}

impl RecordingLogger {
    pub(super) fn matching(&self, level: &str, needle: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .filter(|(recorded, text)| recorded == level && text.contains(needle))
            .count()
    }
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("DEBUG".to_owned(), payload_text(payload)));
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("ERROR".to_owned(), payload_text(payload)));
        true
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub(super) struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

fn payload_text(payload: &Payload) -> String {
    match payload {
        Payload::String(text) => text.clone(),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => String::new(),
    }
}

type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;

fn hooks() -> Hooks {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor")),
    }
}

#[derive(Default)]
pub(super) struct RecordingCache {
    pub(super) failure: bool,
    pub(super) sets: Mutex<Vec<Stored>>,
}

impl RecordingCache {
    pub(super) fn sets(&self) -> Vec<Stored> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CacheBackend for RecordingCache {
    fn default_ttl(&self) -> i64 {
        60
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        if self.failure {
            return Err(BackendError::Unsupported("async_set_cache"));
        }
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RecordingStore {
    pub(super) failure: bool,
    pub(super) writes: Mutex<Vec<PipelineWrite>>,
}

impl RecordingStore {
    pub(super) fn writes(&self) -> Vec<PipelineWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CacheBackend for RecordingStore {
    fn default_ttl(&self) -> i64 {
        120
    }

    async fn async_set_cache_pipeline(
        &self,
        cache_list: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cache_list, ttl));
        if self.failure {
            return Err(BackendError::Unsupported("async_set_cache_pipeline"));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RecordingNative {
    pub(super) failure: bool,
    pub(super) skip_last: bool,
    pub(super) calls: Mutex<usize>,
    pub(super) requests: Mutex<Vec<NativeCacheRequest>>,
    pub(super) responses: Mutex<Vec<CacheValue>>,
}

impl RecordingNative {
    pub(super) fn requests(&self) -> Vec<NativeCacheRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn responses(&self) -> Vec<CacheValue> {
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl NativeEmbeddingStore for RecordingNative {
    fn request(&self, cache_key: &str, _request: &EnvelopeRequest) -> Option<NativeCacheRequest> {
        let skip = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            *calls = calls.saturating_add(1);
            self.skip_last && *calls > 1
        };
        if skip {
            return None;
        }
        Some(NativeCacheRequest {
            cache_key: cache_key.to_owned(),
        })
    }

    async fn async_store_batch(
        &self,
        requests: Vec<NativeCacheRequest>,
        responses: Vec<CacheValue>,
    ) -> CacheResult<()> {
        if self.failure {
            return Err(BackendError::Unsupported("async_store_batch"));
        }
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(requests);
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(responses);
        Ok(())
    }
}

fn key_settings(mode: CacheMode) -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

pub(super) fn settings(ttl: Option<f64>) -> EnvelopeSettings {
    EnvelopeSettings {
        ttl,
        redis_flush_size: None,
    }
}

pub(super) type TestCache = ResponseCache<RecordingLogger, RecordingConsole>;

pub(super) fn cache<B>(backend: Arc<B>, mode: CacheMode, settings: EnvelopeSettings) -> TestCache
where
    B: CacheBackend + 'static,
{
    ResponseCache {
        backend: Arc::new(StoredBackend { backend }),
        key_settings: key_settings(mode),
        settings,
        hooks: hooks(),
        clock: Arc::new(|| NOW),
    }
}

pub(super) fn request(params: Vec<KeyParam>) -> EnvelopeRequest {
    EnvelopeRequest {
        cache_key: None,
        key: CacheKeyRequest {
            params,
            metadata: None,
            litellm_metadata: None,
            litellm_params: None,
            cache_control: None,
        },
        control: DynamicCacheControl::default(),
    }
}

pub(super) fn param(name: &str, value: KeyValue) -> KeyParam {
    KeyParam {
        name: name.to_owned(),
        value,
    }
}

pub(super) fn text(value: &str) -> KeyValue {
    KeyValue::String(value.to_owned())
}

pub(super) fn inputs(values: &[&str]) -> KeyValue {
    KeyValue::Array(values.iter().map(|value| text(value)).collect())
}

pub(super) fn response(
    count: usize,
    prompt_tokens: Option<i64>,
    details: Option<Extra>,
) -> EmbeddingResponse {
    EmbeddingResponse {
        model: Some("text-embedding-3-small".to_owned()),
        data: (0..count)
            .map(|index| EmbeddingRecord {
                embedding: Some(EmbeddingPayload::Vector(vec![Number::from(
                    u64::try_from(index).expect("a small index"),
                )])),
                index: Some(i64::try_from(index).expect("a small index")),
                object: Some("embedding".to_owned()),
            })
            .collect(),
        usage: Some(EmbeddingUsage {
            prompt_tokens,
            prompt_tokens_details: details,
        }),
    }
}

pub(super) fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn decimal(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite fixture"))
}

pub(super) fn details(pairs: &[(&str, Json)]) -> Extra {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn cached(entry: &EmbeddingCacheEntry) -> CachedEmbedding {
    CacheCodec::decode(Some(&entry.cached_embedding)).expect("the entry encodes a cached embedding")
}

pub(super) fn assert_envelope(entry: &EmbeddingCacheEntry) {
    match &entry.cached_data {
        CacheValue::Object(fields) => {
            assert_eq!(fields.get(TIMESTAMP_FIELD), Some(&CacheValue::Float(NOW)));
            assert_eq!(fields.get(RESPONSE_FIELD), Some(&entry.cached_embedding));
        }
        CacheValue::Credential(_)
        | CacheValue::Bytes(_)
        | CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Text(_)
        | CacheValue::Array(_) => panic!("the entry is not an envelope"),
    }
}
