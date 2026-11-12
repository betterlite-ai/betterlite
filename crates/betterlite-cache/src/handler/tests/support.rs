use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde_json::Number;

use crate::backend::memory::local::LocalMemoryOps;
use crate::backend::{BackendError, CacheResult, Ttl};
use crate::cache_key::request::{CacheKeyRequest, KeyParam};
use crate::cache_key::settings::CacheKeySettings;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::dual_cache::{DualCache, DualCacheSettings};
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use crate::envelope::lookup::CacheLookup;
use crate::handler::retrieve::{AsyncSupport, CacheSource};
use crate::handler::state::{CachingHandler, HandlerDependencies, OriginalFunction};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::semantic::SemanticCacheScope;

pub(crate) const FIXTURE_NOW: f64 = 1_700_000_000.0;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadKind {
    Sync,
    Async,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Read {
    pub(crate) kind: ReadKind,
    pub(crate) key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadSupport {
    Both,
    SyncOnly,
    AsyncOnly,
}

pub(crate) struct RecordingBackend {
    stored: Mutex<BTreeMap<String, CacheValue>>,
    reads: Mutex<Vec<Read>>,
    sync_enabled: bool,
    async_enabled: bool,
}

impl RecordingBackend {
    pub(crate) fn new(support: ReadSupport) -> Self {
        Self {
            stored: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            sync_enabled: matches!(support, ReadSupport::Both | ReadSupport::SyncOnly),
            async_enabled: matches!(support, ReadSupport::Both | ReadSupport::AsyncOnly),
        }
    }

    pub(crate) fn seed(&self, key: &str, value: CacheValue) {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.insert(key.to_owned(), value);
    }

    pub(crate) fn clear(&self) {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.clear();
    }

    pub(crate) fn reads(&self) -> Vec<Read> {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.clone()
    }

    fn record(&self, kind: ReadKind, key: &str) {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.push(Read {
            kind,
            key: key.to_owned(),
        });
    }

    fn read(&self, key: &str) -> Option<CacheValue> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.get(key).cloned()
    }

    fn write(&self, key: &str, value: &CacheValue) {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.insert(key.to_owned(), value.clone());
    }
}

#[async_trait]
impl EnvelopeBackend for RecordingBackend {
    fn get_cache(&self, key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        self.record(ReadKind::Sync, key);
        if !self.sync_enabled {
            return Ok(None);
        }
        Ok(self.read(key))
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.write(key, &value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        self.record(ReadKind::Async, key);
        if !self.async_enabled {
            return Err(BackendError::Unsupported("async_get_cache"));
        }
        Ok(self.read(key))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.write(key, &value);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.write(key, &value);
        Ok(())
    }
}

pub(crate) type TestCache = ResponseCache<SilentLogger, SilentConsole>;

pub(crate) fn hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    }
}

pub(crate) fn key_settings() -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode: CacheMode::DefaultOn,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

pub(crate) fn cache(backend: Arc<RecordingBackend>) -> TestCache {
    let clock: Clock = Arc::new(|| FIXTURE_NOW);
    ResponseCache {
        backend,
        key_settings: key_settings(),
        settings: EnvelopeSettings {
            ttl: None,
            redis_flush_size: None,
        },
        hooks: hooks(),
        clock,
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

pub(crate) fn memory() -> Arc<LocalMemoryOps> {
    let clock: Clock = Arc::new(|| FIXTURE_NOW);
    let operations = LocalMemoryOps::new(1024, clock).expect("the memory tier builds");
    Arc::new(operations)
}

pub(crate) fn handler_named(name: &str, parameters: &[&str], kwargs: KeyObject) -> CachingHandler {
    CachingHandler::new(
        HandlerDependencies {
            original_function: OriginalFunction {
                name: name.to_owned(),
                parameters: parameters.iter().copied().map(str::to_owned).collect(),
            },
            start_time: 0.0,
            redis_cache: None,
            in_memory_cache: memory(),
            dual_settings: dual_settings(),
        },
        kwargs,
    )
}

pub(crate) fn handler(kwargs: KeyObject) -> CachingHandler {
    handler_named("acompletion", &["model", "messages"], kwargs)
}

pub(crate) fn dual_cache() -> DualCache<LocalMemoryOps> {
    DualCache::new(memory(), None, dual_settings())
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

pub(crate) fn integer(value: i64) -> KeyValue {
    KeyValue::Number(Number::from(value))
}

pub(crate) fn float(value: f64) -> KeyValue {
    KeyValue::Number(Number::from_f64(value).expect("the fixture float is finite"))
}

pub(crate) fn cache_text(value: &str) -> CacheValue {
    CacheValue::Text(value.to_owned())
}

pub(crate) fn cache_object(entries: &[(&str, CacheValue)]) -> CacheValue {
    CacheValue::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

pub(crate) fn key_request_of(entries: &[(&str, KeyValue)]) -> CacheKeyRequest {
    CacheKeyRequest {
        params: entries
            .iter()
            .map(|(name, value)| KeyParam {
                name: (*name).to_owned(),
                value: value.clone(),
            })
            .collect(),
        metadata: None,
        litellm_metadata: None,
        litellm_params: None,
        cache_control: None,
    }
}

pub(crate) fn envelope(response: CacheValue) -> CacheValue {
    CacheValue::Object(BTreeMap::from([
        ("response".to_owned(), response),
        ("timestamp".to_owned(), CacheValue::Float(0.0)),
    ]))
}

pub(crate) const MODEL: &str = "gpt-4";

pub(crate) fn chat_kwargs() -> KeyObject {
    kwargs(&[
        ("model", text(MODEL)),
        ("messages", KeyValue::Array(Vec::new())),
    ])
}

pub(crate) fn embedding_kwargs(inputs: &[&str]) -> KeyObject {
    kwargs(&[
        ("model", text("text-embedding-3-small")),
        (
            "input",
            KeyValue::Array(inputs.iter().map(|value| text(value)).collect()),
        ),
    ])
}

pub(crate) fn cached_embedding_fields(format_version: i64) -> BTreeMap<String, CacheValue> {
    BTreeMap::from([
        (
            "embedding".to_owned(),
            CacheValue::Array(vec![CacheValue::Float(0.5), CacheValue::Float(-1.25)]),
        ),
        ("index".to_owned(), CacheValue::Integer(0)),
        (
            "object".to_owned(),
            CacheValue::Text("embedding".to_owned()),
        ),
        (
            "model".to_owned(),
            CacheValue::Text("text-embedding-3-small".to_owned()),
        ),
        ("prompt_tokens".to_owned(), CacheValue::Integer(3)),
        ("prompt_tokens_details".to_owned(), CacheValue::Null),
        (
            "format_version".to_owned(),
            CacheValue::Integer(format_version),
        ),
    ])
}

pub(crate) fn cached_embedding(format_version: i64) -> CacheValue {
    CacheValue::Object(cached_embedding_fields(format_version))
}

pub(crate) fn source(
    cache: &TestCache,
    async_support: AsyncSupport,
) -> CacheSource<'_, SilentLogger, SilentConsole> {
    CacheSource {
        cache,
        async_support,
    }
}
