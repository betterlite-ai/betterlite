#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

use crate::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use crate::cache_key::request::{CacheKeyRequest, KeyParam};
use crate::cache_key::settings::CacheKeySettings;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::facade::{EnvelopeSettings, ResponseCache};
use crate::envelope::lookup::{CacheLookup, SEMANTIC_SIMILARITY};
use crate::envelope::request::EnvelopeRequest;
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::control::DynamicCacheControl;
use crate::types::semantic::SemanticCacheScope;

pub(crate) const NOW: f64 = 1_700_000_000.0;

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) records: Mutex<Vec<Payload>>,
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub(crate) struct RecordingConsole {
    pub(crate) prints: Mutex<Vec<Payload>>,
}

impl ConsoleSink for RecordingConsole {
    fn print(&self, payload: &Payload) -> bool {
        self.prints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }
}

pub(crate) type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;

pub(crate) fn hooks() -> Hooks {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole::default(),
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles"),
        ),
    }
}

pub(crate) fn messages(hooks: &Hooks) -> Vec<String> {
    let records = hooks
        .logger
        .records
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    records.iter().filter_map(payload_text).collect()
}

fn payload_text(payload: &Payload) -> Option<String> {
    match payload {
        Payload::String(text) => Some(text.clone()),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => None,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Failure {
    #[default]
    None,
    Get,
    Set,
    Batch,
}

#[derive(Default)]
pub(crate) struct BackendState {
    gets: Vec<(String, CacheLookup)>,
    sets: Vec<(String, CacheValue, Ttl)>,
    async_sets: Vec<(String, CacheValue, Ttl)>,
    batches: Vec<(String, CacheValue, Ttl)>,
}

pub(crate) struct FakeBackend {
    pub(crate) get_result: Option<CacheValue>,
    pub(crate) stamp: Option<f64>,
    pub(crate) failure: Failure,
    state: Mutex<BackendState>,
}

impl FakeBackend {
    pub(crate) fn new(get_result: Option<CacheValue>) -> Self {
        Self {
            get_result,
            stamp: None,
            failure: Failure::None,
            state: Mutex::new(BackendState::default()),
        }
    }

    pub(crate) fn gets(&self) -> Vec<(String, CacheLookup)> {
        self.state()
            .gets
            .iter()
            .map(|(key, lookup)| (key.clone(), lookup.clone()))
            .collect()
    }

    pub(crate) fn sets(&self) -> Vec<(String, CacheValue, Ttl)> {
        self.state().sets.clone()
    }

    pub(crate) fn async_sets(&self) -> Vec<(String, CacheValue, Ttl)> {
        self.state().async_sets.clone()
    }

    pub(crate) fn batches(&self) -> Vec<(String, CacheValue, Ttl)> {
        self.state().batches.clone()
    }

    fn state(&self) -> MutexGuard<'_, BackendState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stamp_lookup(&self, lookup: &mut CacheLookup) {
        let (Some(stamp), Some(metadata)) = (self.stamp, lookup.metadata.as_mut()) else {
            return;
        };
        let Some(number) = serde_json::Number::from_f64(stamp) else {
            return;
        };
        metadata.insert(SEMANTIC_SIMILARITY.to_owned(), KeyValue::Number(number));
    }

    fn record_get(&self, key: &str, lookup: &mut CacheLookup) -> CacheResult<()> {
        self.stamp_lookup(lookup);
        self.state().gets.push((key.to_owned(), lookup.clone()));
        if self.failure == Failure::Get {
            return Err(BackendError::Unsupported("get_cache"));
        }
        Ok(())
    }

    fn record_set(state: &mut BackendState, key: &str, value: &CacheValue, ttl: Ttl) {
        state.sets.push((key.to_owned(), value.clone(), ttl));
    }
}

#[async_trait]
impl EnvelopeBackend for FakeBackend {
    fn get_cache(&self, key: &str, lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        self.record_get(key, lookup)?;
        Ok(self.get_result.clone())
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        Self::record_set(&mut self.state(), key, &value, ttl);
        if self.failure == Failure::Set {
            return Err(BackendError::Unsupported("set_cache"));
        }
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        self.record_get(key, lookup)?;
        Ok(self.get_result.clone())
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state().async_sets.push((key.to_owned(), value, ttl));
        if self.failure == Failure::Set {
            return Err(BackendError::Unsupported("async_set_cache"));
        }
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state().batches.push((key.to_owned(), value, ttl));
        if self.failure == Failure::Batch {
            return Err(BackendError::Unsupported("batch_cache_write"));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct BatchStore {
    batches: Mutex<Vec<(String, CacheValue, Ttl)>>,
}

impl BatchStore {
    pub(crate) fn batches(&self) -> Vec<(String, CacheValue, Ttl)> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CacheBackend for BatchStore {
    fn default_ttl(&self) -> i64 {
        60
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
        Ok(())
    }
}

pub(crate) fn key_settings(mode: CacheMode) -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

pub(crate) fn cache(
    backend: Arc<dyn EnvelopeBackend>,
    key_settings: CacheKeySettings,
    settings: EnvelopeSettings,
) -> ResponseCache<RecordingLogger, RecordingConsole> {
    ResponseCache {
        backend,
        key_settings,
        settings,
        hooks: hooks(),
        clock: Arc::new(|| NOW),
    }
}

pub(crate) fn fake_cache(
    fake: &Arc<FakeBackend>,
    key_settings: CacheKeySettings,
    settings: EnvelopeSettings,
) -> ResponseCache<RecordingLogger, RecordingConsole> {
    let plain = Arc::clone(fake);
    cache(plain, key_settings, settings)
}

// joined is the s-maxage key and hyphenated is the s-max-age key.
pub(crate) fn control(
    ttl: Option<i64>,
    joined: Option<i64>,
    hyphenated: Option<i64>,
) -> DynamicCacheControl {
    DynamicCacheControl {
        ttl,
        namespace: None,
        s_maxage: joined,
        s_max_age: hyphenated,
        no_cache: None,
        no_store: None,
    }
}

pub(crate) fn empty_request() -> EnvelopeRequest {
    EnvelopeRequest {
        cache_key: None,
        key: CacheKeyRequest {
            params: Vec::new(),
            metadata: None,
            litellm_metadata: None,
            litellm_params: None,
            cache_control: None,
        },
        control: DynamicCacheControl::default(),
    }
}

pub(crate) fn with_params(params: Vec<KeyParam>) -> EnvelopeRequest {
    let mut request = empty_request();
    request.key.params = params;
    request
}

pub(crate) fn number(value: f64) -> KeyValue {
    KeyValue::Number(serde_json::Number::from_f64(value).expect("a finite number"))
}

pub(crate) fn object(pairs: &[(&str, KeyValue)]) -> KeyObject {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn envelope(timestamp: Option<f64>, response: CacheValue) -> CacheValue {
    let mut fields = BTreeMap::new();
    if let Some(seconds) = timestamp {
        fields.insert(TIMESTAMP_FIELD.to_owned(), CacheValue::Float(seconds));
    }
    fields.insert(RESPONSE_FIELD.to_owned(), response);
    CacheValue::Object(fields)
}

pub(crate) fn reply() -> CacheValue {
    envelope(
        Some(NOW),
        CacheValue::Object(BTreeMap::from([(
            "choices".to_owned(),
            CacheValue::Array(Vec::new()),
        )])),
    )
}

pub(crate) fn reply_response() -> CacheValue {
    CacheValue::Object(BTreeMap::from([(
        "choices".to_owned(),
        CacheValue::Array(Vec::new()),
    )]))
}
