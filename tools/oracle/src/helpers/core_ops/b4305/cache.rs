use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::facade::{EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::lookup::CacheLookup as EnvelopeLookup;
use betterlite_cache::handler::embedding::{embedding_base_request, embedding_entry_key};
use betterlite_cache::handler::kwargs::key_request;
use betterlite_cache::types::cache_type::CacheMode;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

use super::input::{Scenario, Seed};

pub(crate) struct Silent;

impl LoggerSink for Silent {
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

impl ConsoleSink for Silent {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

#[derive(Default)]
pub(crate) struct SeededBackend {
    stored: Mutex<BTreeMap<String, CacheValue>>,
}

impl SeededBackend {
    fn insert(&self, key: String, value: CacheValue) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, value);
    }

    fn read(&self, key: &str) -> Option<CacheValue> {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl EnvelopeBackend for SeededBackend {
    fn get_cache(
        &self,
        key: &str,
        _lookup: &mut EnvelopeLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(self.read(key))
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.insert(key.to_owned(), value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut EnvelopeLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(self.read(key))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.insert(key.to_owned(), value);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.insert(key.to_owned(), value);
        Ok(())
    }
}

// A stored value sits under the literal key, under the embedding entry key for one input, or under the key the request derives
fn seed_key(
    cache: &ResponseCache<Silent, Silent>,
    scenario: &Scenario,
    seed: &Seed,
) -> Result<String, Box<HelperError>> {
    if let Some(key) = &seed.key {
        return Ok(key.clone());
    }
    if let Some(input) = &seed.input {
        let base = embedding_base_request(&scenario.kwargs).or_value_error()?;
        return embedding_entry_key(cache, &base, input).or_value_error();
    }
    let mut request = key_request(&scenario.kwargs).or_value_error()?;
    get_cache_key(&mut request, &cache.key_settings, &cache.hooks).or_value_error()
}

pub(crate) fn cache_fixture(
    scenario: &Scenario,
) -> Result<ResponseCache<Silent, Silent>, Box<HelperError>> {
    let backend = Arc::new(SeededBackend::default());
    let stored: Arc<dyn EnvelopeBackend> = Arc::clone(&backend);
    let cache = ResponseCache {
        backend: stored,
        key_settings: CacheKeySettings {
            cache_type: scenario.cache_type,
            mode: CacheMode::DefaultOn,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: EnvelopeSettings::default(),
        hooks: VerboseHooks {
            logger: Silent,
            console: Silent,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?),
        },
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    for seed in &scenario.entries {
        backend.insert(seed_key(&cache, scenario, seed)?, seed.value.clone());
    }
    Ok(cache)
}
