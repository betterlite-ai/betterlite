use std::sync::Arc;

use betterlite_cache::backend::CacheBackend;
use betterlite_cache::backend::memory::MemoryCache;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::configuration::backend_construction::BackendConstructionError;
use betterlite_cache::disk::Clock;
use betterlite_cache::embedding::embedding_cache::add_embedding_entry_to_cache;
use betterlite_cache::envelope::backend::{EnvelopeBackend, StoredBackend};
use betterlite_cache::envelope::decode::decode_cached;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use pyo3::prelude::*;

use super::convert;
use super::error::CacheError;
use super::settings::CacheSettingsWire;

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
    let ttl = convert::request_ttl(&envelope, effective_ttl(&wire));
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
    let decoded =
        decode_cached(cached.as_ref(), max_age, convert::wall_clock()).map_err(CacheError::from)?;
    Ok(convert::value_text(&decoded)?)
}

// The settings-only twin of NativeCache::write_embedding: one input's entry, with Rust's per-item rules.
#[pyfunction]
pub(crate) fn cache_embedding(settings: &str, request: &str) -> PyResult<String> {
    let wire = convert::parse_settings(settings)?;
    let call = convert::parse_call(request)?;
    let mut envelope = call.envelope_request()?;
    let clock: Clock = Arc::new(convert::wall_clock);
    // The entry rules read the facade's key settings and clock and store nothing, so this in-memory backend never answers.
    let operations = LocalMemoryOps::new(1024_i64, Arc::clone(&clock)).map_err(|source| {
        CacheError::Construction {
            source: Box::new(BackendConstructionError::Memory { source }),
        }
    })?;
    let stored: Arc<dyn CacheBackend> = Arc::new(MemoryCache { operations });
    let backend: Arc<dyn EnvelopeBackend> = Arc::new(StoredBackend { backend: stored });
    let facade = convert::facade(&wire, backend, effective_ttl(&wire), clock)?;
    let result = call.embedding_result()?;
    let entry = add_embedding_entry_to_cache(
        &facade,
        &mut envelope,
        &result,
        call.input_value()?,
        call.result_index(),
    )
    .map_err(CacheError::from)?;
    Ok(convert::embedding_text(&entry)?)
}

// Cache::open's ttl rule, for the two entry points that answer without opening a backend: a per-kind default wins over ttl.
fn effective_ttl(wire: &CacheSettingsWire) -> Option<f64> {
    match wire.cache_type {
        LiteLlmCacheType::Local => wire.default_in_memory_ttl.or(wire.ttl),
        LiteLlmCacheType::Redis
        | LiteLlmCacheType::RedisSemantic
        | LiteLlmCacheType::ValkeySemantic => wire.default_in_redis_ttl.or(wire.ttl),
        LiteLlmCacheType::S3
        | LiteLlmCacheType::Disk
        | LiteLlmCacheType::QdrantSemantic
        | LiteLlmCacheType::AzureBlob
        | LiteLlmCacheType::Gcs => wire.ttl,
    }
}
