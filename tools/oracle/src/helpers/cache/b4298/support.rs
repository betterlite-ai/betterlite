use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use betterlite_cache::cache_key::request::CacheKeyRequest;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::embedding::embedding_cache::pipeline::{
    NativeCacheRequest, NativeEmbeddingStore,
};
use betterlite_cache::envelope::backend::StoredBackend;
use betterlite_cache::envelope::facade::{EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::request::EnvelopeRequest;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

pub(crate) const NOW: f64 = 1_700_000_000.0;

pub(crate) type Stored = (String, CacheValue, Ttl);
pub(crate) type PipelineWrite = (Vec<(String, CacheValue)>, Ttl);
pub(crate) type HelperCache = ResponseCache<RecordingLogger, RecordingConsole>;

#[derive(Default)]
pub(crate) struct RecordingBackend {
    pub(crate) failure: bool,
    pub(crate) sets: Mutex<Vec<Stored>>,
}

impl RecordingBackend {
    pub(crate) fn sets(&self) -> Vec<Stored> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, key: &str, value: CacheValue, ttl: Ttl) {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
    }
}

#[async_trait]
impl CacheBackend for RecordingBackend {
    fn default_ttl(&self) -> i64 {
        60
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        if self.failure {
            return Err(BackendError::Unsupported("async_set_cache"));
        }
        self.record(key, value, ttl);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingStore {
    pub(crate) writes: Mutex<Vec<PipelineWrite>>,
}

impl RecordingStore {
    pub(crate) fn writes(&self) -> Vec<PipelineWrite> {
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
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.push((cache_list, ttl));
        drop(writes);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingNative {
    pub(crate) failure: bool,
    pub(crate) skip_after_first: bool,
    pub(crate) called: Mutex<bool>,
    pub(crate) calls: Mutex<usize>,
    pub(crate) requests: Mutex<Vec<NativeCacheRequest>>,
    pub(crate) responses: Mutex<Vec<CacheValue>>,
}

impl RecordingNative {
    pub(crate) fn requests(&self) -> Vec<NativeCacheRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn responses(&self) -> Vec<CacheValue> {
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
            self.skip_after_first && *calls > 1
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
        *self.called.lock().unwrap_or_else(PoisonError::into_inner) = true;
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

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) records: Mutex<Vec<(String, String)>>,
}

pub(crate) trait AsCacheStore {
    fn as_cache_store(&self) -> &dyn CacheBackend;
}

impl AsCacheStore for RecordingStore {
    fn as_cache_store(&self) -> &dyn CacheBackend {
        self
    }
}

pub(crate) trait AsNativeStore {
    fn as_native_store(&self) -> &dyn NativeEmbeddingStore;
}

impl AsNativeStore for RecordingNative {
    fn as_native_store(&self) -> &dyn NativeEmbeddingStore {
        self
    }
}

impl RecordingLogger {
    pub(crate) fn matching(&self, level: &str, marker: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .filter(|(recorded, text)| recorded == level && text.contains(marker))
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

pub(crate) struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

pub(crate) fn cache(
    backend: Arc<RecordingBackend>,
    cache_type: Option<LiteLlmCacheType>,
    mode: Option<CacheMode>,
    settings: &EnvelopeSettings,
) -> Result<HelperCache, Box<crate::helpers::HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    Ok(ResponseCache {
        backend: Arc::new(StoredBackend { backend }),
        key_settings: CacheKeySettings {
            cache_type: cache_type.unwrap_or(LiteLlmCacheType::Local),
            mode: mode.unwrap_or(CacheMode::DefaultOn),
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: settings.clone(),
        hooks: VerboseHooks {
            logger: RecordingLogger::default(),
            console: RecordingConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(redactor),
        },
        clock: Arc::new(|| NOW),
    })
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(failure)
}

pub(crate) fn rebuild(request: &EnvelopeRequest) -> EnvelopeRequest {
    EnvelopeRequest {
        cache_key: request.cache_key.clone(),
        key: CacheKeyRequest {
            params: request.key.params.clone(),
            metadata: request.key.metadata.clone(),
            litellm_metadata: request.key.litellm_metadata.clone(),
            litellm_params: request.key.litellm_params.clone(),
            cache_control: request.key.cache_control.clone(),
        },
        control: request.control.clone(),
    }
}

pub(crate) fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
