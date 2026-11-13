#[path = "b4297/cases.rs"]
mod cases;
#[path = "b4297/output.rs"]
mod output;

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::cache_key::request::CacheKeyRequest;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::lookup::{CacheLookup, SEMANTIC_SIMILARITY};
use betterlite_cache::envelope::request::EnvelopeRequest;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde::Deserialize;

const FIXTURE_NOW: f64 = 1_700_000_000.0;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    GetCacheLogic(GetCacheLogicInput),
    GetCache(GetCacheInput),
    AddCache(WriteInput),
    AsyncAddCache(WriteInput),
}

#[derive(Deserialize)]
pub(crate) struct GetCacheLogicInput {
    pub(crate) cached: Option<CacheValue>,
    pub(crate) max_age: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct GetCacheInput {
    pub(crate) request: EnvelopeRequest,
    pub(crate) cached: Option<CacheValue>,
    pub(crate) stamp: Option<f64>,
}

#[derive(Deserialize)]
pub(crate) struct WriteInput {
    pub(crate) request: EnvelopeRequest,
    pub(crate) result: CacheValue,
    #[serde(default)]
    pub(crate) settings: EnvelopeSettings,
    pub(crate) cache_type: Option<LiteLlmCacheType>,
}

#[derive(Clone)]
pub(crate) struct RecordedWrite {
    pub(crate) method: &'static str,
    pub(crate) key: String,
    pub(crate) value: CacheValue,
    pub(crate) ttl: Ttl,
}

#[derive(Default)]
pub(crate) struct Recorded {
    lookup: Option<CacheLookup>,
    writes: Vec<RecordedWrite>,
}

pub(crate) struct Snapshot {
    pub(crate) lookup: Option<CacheLookup>,
    pub(crate) write: Option<RecordedWrite>,
}

pub(crate) struct RecordingBackend {
    cached: Option<CacheValue>,
    stamp: Option<f64>,
    recorded: Mutex<Recorded>,
}

impl RecordingBackend {
    pub(crate) fn new(cached: Option<CacheValue>, stamp: Option<f64>) -> Self {
        Self {
            cached,
            stamp,
            recorded: Mutex::new(Recorded::default()),
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        let lookup = recorded.lookup.clone();
        let write = recorded.writes.last().cloned();
        drop(recorded);
        Snapshot { lookup, write }
    }

    fn record_lookup(&self, lookup: &mut CacheLookup) {
        if let (Some(stamp), Some(metadata)) = (self.stamp, lookup.metadata.as_mut())
            && let Some(number) = serde_json::Number::from_f64(stamp)
        {
            metadata.insert(SEMANTIC_SIMILARITY.to_owned(), KeyValue::Number(number));
        }
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.lookup = Some(lookup.clone());
    }

    fn record_write(&self, method: &'static str, key: &str, value: &CacheValue, ttl: Ttl) {
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.writes.push(RecordedWrite {
            method,
            key: key.to_owned(),
            value: value.clone(),
            ttl,
        });
    }
}

#[async_trait]
impl EnvelopeBackend for RecordingBackend {
    fn get_cache(&self, _key: &str, lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        self.record_lookup(lookup);
        Ok(self.cached.clone())
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.record_write("set_cache", key, &value, ttl);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        _key: &str,
        lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        self.record_lookup(lookup);
        Ok(self.cached.clone())
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.record_write("async_set_cache", key, &value, ttl);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.record_write("batch_cache_write", key, &value, ttl);
        Ok(())
    }
}

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

pub(crate) type HelperCache = ResponseCache<SilentLogger, SilentConsole>;

pub(crate) fn cache(
    backend: Arc<RecordingBackend>,
    settings: &EnvelopeSettings,
    cache_type: Option<LiteLlmCacheType>,
) -> Result<HelperCache, Box<crate::helpers::HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let clock: Clock = Arc::new(|| FIXTURE_NOW);
    Ok(ResponseCache {
        backend,
        key_settings: CacheKeySettings {
            cache_type: cache_type.unwrap_or(LiteLlmCacheType::Local),
            mode: CacheMode::DefaultOn,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: settings.clone(),
        hooks: VerboseHooks {
            logger: SilentLogger,
            console: SilentConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(redactor),
        },
        clock,
    })
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(failure)
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

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::GetCacheLogic(case) => cases::get_cache_logic(case),
            Input::GetCache(case) => cases::get_cache(case),
            Input::AddCache(case) => cases::add_cache(case),
            Input::AsyncAddCache(case) => cases::async_add_cache(case),
        }
    }
}
