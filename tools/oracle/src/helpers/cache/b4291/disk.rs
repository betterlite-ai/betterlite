use std::sync::Arc;

use betterlite_cache::disk::{Clock, DiskCache};
use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;

use super::super::b4290::failure;
use crate::helpers::HelperResult;

const PROBE_CLOCK_SECONDS: f64 = 1_000_000.0;

const ABSENT_KEY: &str = "absent-key";

#[derive(Deserialize)]
pub(crate) struct TtlEntry {
    pub(crate) key: String,
    pub(crate) value: CacheValue,
    pub(crate) ttl: f64,
}

#[derive(Deserialize)]
pub(crate) struct StoredEntry {
    pub(crate) key: String,
    pub(crate) value: CacheValue,
}

fn probe() -> Result<(TempDir, DiskCache), Box<crate::helpers::HelperError>> {
    let directory = TempDir::new().map_err(failure)?;
    let clock: Clock = Arc::new(|| PROBE_CLOCK_SECONDS);
    let cache = DiskCache::open(Some(directory.path()), clock).map_err(failure)?;
    Ok((directory, cache))
}

pub(crate) fn set_and_get(key: &str, value: &CacheValue) -> HelperResult {
    let (_directory, cache) = probe()?;
    cache.set_cache(key, value, None).map_err(failure)?;
    let read = cache.get_cache(key).map_err(failure)?;
    let missing = cache.get_cache(ABSENT_KEY).map_err(failure)?;
    Ok(json!({"read": read, "missing": missing}))
}

pub(crate) fn ttl_expiry(entries: &[TtlEntry]) -> HelperResult {
    let (_directory, cache) = probe()?;
    let mut reads = Vec::with_capacity(entries.len());
    for entry in entries {
        cache
            .set_cache(&entry.key, &entry.value, Some(entry.ttl))
            .map_err(failure)?;
        reads.push(cache.get_cache(&entry.key).map_err(failure)?);
    }
    Ok(json!({"reads": reads}))
}

pub(crate) fn truthy_reads(values: &[CacheValue]) -> HelperResult {
    let (_directory, cache) = probe()?;
    let mut reads = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let key = format!("key-{index}");
        cache.set_cache(&key, value, None).map_err(failure)?;
        reads.push(cache.get_cache(&key).map_err(failure)?);
    }
    Ok(json!({"reads": reads}))
}

pub(crate) fn batch_get(entries: &[StoredEntry], keys: &[String]) -> HelperResult {
    let (_directory, cache) = probe()?;
    for entry in entries {
        cache
            .set_cache(&entry.key, &entry.value, None)
            .map_err(failure)?;
    }
    let reads = cache.batch_get_cache(keys).map_err(failure)?;
    Ok(json!({"reads": reads}))
}

pub(crate) fn increment(
    key: &str,
    initial: Option<&CacheValue>,
    increments: &[i64],
) -> HelperResult {
    let (_directory, cache) = probe()?;
    if let Some(value) = initial {
        cache.set_cache(key, value, None).map_err(failure)?;
    }
    let mut results = Vec::with_capacity(increments.len());
    for step in increments {
        results.push(cache.increment_cache(key, *step, None).map_err(failure)?);
    }
    let stored = cache.get_cache(key).map_err(failure)?;
    Ok(json!({"results": results, "stored": stored}))
}

pub(crate) fn delete(key: &str, value: &CacheValue, kept: &StoredEntry) -> HelperResult {
    let (_directory, cache) = probe()?;
    cache.set_cache(key, value, None).map_err(failure)?;
    cache
        .set_cache(&kept.key, &kept.value, None)
        .map_err(failure)?;
    cache.delete_cache(key).map_err(failure)?;
    cache.delete_cache(ABSENT_KEY).map_err(failure)?;
    let deleted = cache.get_cache(key).map_err(failure)?;
    let still_kept = cache.get_cache(&kept.key).map_err(failure)?;
    Ok(json!({"deleted": deleted, "kept": still_kept}))
}
