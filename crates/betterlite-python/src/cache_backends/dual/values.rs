use std::sync::Arc;

use betterlite_cache::dual_cache::{CacheOptions, Ttl};
use betterlite_models::decisions::Json;
use pyo3::prelude::*;

use super::MemoryDual;
use crate::cache_backends::convert;
use crate::cache_backends::error::BackendHandleError;

pub(super) const fn options(ttl: Option<f64>) -> CacheOptions {
    CacheOptions {
        ttl,
        local_only: false,
        refresh_ttl: false,
    }
}

pub(super) fn text_of_ttl(ttl: Option<Ttl>) -> PyResult<Option<String>> {
    ttl.map(|value| {
        serde_json::to_string(&value).map_err(|source| BackendHandleError::Encode { source })
    })
    .transpose()
    .map_err(PyErr::from)
}

pub(super) async fn store(
    cache: Arc<MemoryDual>,
    key: String,
    json: Json,
    ttl: Option<f64>,
) -> PyResult<()> {
    cache.async_set_cache(&key, &json, options(ttl)).await;
    Ok(())
}

pub(super) fn answer_json(stored: Option<Json>) -> PyResult<Option<String>> {
    stored.map_or_else(
        || Ok(None),
        |value| convert::text_of_json(&value).map(Some).map_err(PyErr::from),
    )
}

pub(super) async fn count(
    cache: Arc<MemoryDual>,
    key: String,
    by: f64,
    ttl: Option<f64>,
    memory_ttl: Option<f64>,
) -> PyResult<String> {
    let Some(counted) = cache
        .async_increment_cache(&key, by, options(ttl.or(memory_ttl)))
        .await
    else {
        return Err(PyErr::from(BackendHandleError::IncrementUnavailable));
    };
    convert::number_text(counted).map_err(PyErr::from)
}
