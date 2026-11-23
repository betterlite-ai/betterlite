use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheResult, Ttl};
use betterlite_cache::disk::Clock;
use betterlite_cache::embedding::embedding_cache::add_embedding_entry_to_cache;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::decode::decode_cached;
use betterlite_cache::envelope::lookup::CacheLookup;
use betterlite_context::local_ttl_cache::CacheValue;
use pyo3::prelude::*;

use super::convert;

mod tests;

// The settings-only twin of NativeCache::cache_key, for a Cache whose backend is a Python object.
#[pyfunction]
pub(crate) fn cache_key(settings: &str, request: &str) -> PyResult<String> {
    let wire = convert::parse_settings(settings)?;
    let call = convert::parse_call(request)?;
    let mut envelope = call.envelope_request()?;
    let hooks = convert::hooks()?;
    Ok(convert::resolve_key(
        &mut envelope,
        &wire.key_settings(),
        &hooks,
    )?)
}

// LiteLLM's _add_cache_logic without a backend: the key, the envelope a Python backend stores, and the ttl it applies.
#[pyfunction]
pub(crate) fn cache_envelope(settings: &str, request: &str) -> PyResult<String> {
    let wire = convert::parse_settings(settings)?;
    let call = convert::parse_call(request)?;
    let mut envelope = call.envelope_request()?;
    let hooks = convert::hooks()?;
    let key = convert::resolve_key(&mut envelope, &wire.key_settings(), &hooks)?;
    let result = call.result_value()?;
    let ttl = convert::request_ttl(&envelope, wire.effective_ttl());
    Ok(convert::envelope_text(
        &key,
        convert::wall_clock(),
        &result,
        ttl,
    )?)
}

// LiteLLM's _get_cache_logic over a value a Python backend returned: the age check, then the decoded response.
#[pyfunction]
pub(crate) fn cache_decode(request: &str) -> PyResult<String> {
    let call = convert::parse_call(request)?;
    let cached = call.cached_value()?;
    let max_age = call.max_age()?;
    let decoded = decode_cached(cached.as_ref(), max_age, convert::wall_clock())?;
    Ok(convert::value_text(&decoded)?)
}

// The settings-only twin of NativeCache::write_embedding: one input's entry, with Rust's per-item rules.
#[pyfunction]
pub(crate) fn cache_embedding(settings: &str, request: &str) -> PyResult<String> {
    let wire = convert::parse_settings(settings)?;
    let call = convert::parse_call(request)?;
    let mut envelope = call.envelope_request()?;
    let clock: Clock = Arc::new(convert::wall_clock);
    let backend: Arc<dyn EnvelopeBackend> = Arc::new(NoBackend);
    let facade = convert::facade(&wire, backend, wire.effective_ttl(), clock)?;
    let result = call.embedding_result()?;
    let entry = add_embedding_entry_to_cache(
        &facade,
        &mut envelope,
        &result,
        call.input_value()?,
        call.result_index(),
    )?;
    Ok(convert::embedding_text(&entry)?)
}

// The entry rules read the facade's key settings and clock but store nothing, so this backend never answers.
struct NoBackend;

#[async_trait]
impl EnvelopeBackend for NoBackend {
    fn get_cache(&self, _key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Err(BackendError::Unsupported("get_cache"))
    }

    fn set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Err(BackendError::Unsupported("set_cache"))
    }

    async fn async_get_cache(
        &self,
        _key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Err(BackendError::Unsupported("async_get_cache"))
    }

    async fn async_set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Err(BackendError::Unsupported("async_set_cache"))
    }

    async fn batch_cache_write(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Ttl,
    ) -> CacheResult<()> {
        Err(BackendError::Unsupported("batch_cache_write"))
    }
}
