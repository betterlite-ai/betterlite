use super::FanOutInner;
use crate::otel::config::destination::{CacheKey, OtelDestination};
use indexmap::IndexMap;
use opentelemetry_sdk::trace::SpanProcessor;
use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, PoisonError};
pub(super) const MAX_CACHED_PROCESSORS: usize = 32;
#[derive(Debug)]
pub(super) struct CachedProcessor {
    pub(super) id: u64,
    pub(super) processor: Arc<dyn SpanProcessor>,
}

#[derive(Debug, Default)]
pub(super) struct CacheState {
    pub(super) closed: bool,
    pub(super) processors: IndexMap<CacheKey, Arc<CachedProcessor>>,
    // retired processors stay alive until every span exporting through them has landed
    pub(super) retired: IndexMap<u64, Arc<CachedProcessor>>,
    pub(super) exporting: HashMap<u64, usize>,
    pub(super) next_id: u64,
}

pub(super) fn acquire(
    inner: &FanOutInner,
    destination: &OtelDestination,
) -> Option<Arc<CachedProcessor>> {
    let mut cache = inner.cache.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.closed {
        return None;
    }
    let processor = cached_or_built(inner, &mut cache, destination, true)?;
    let count = cache.exporting.get(&processor.id).copied().unwrap_or(0);
    cache
        .exporting
        .insert(processor.id, count.saturating_add(1));
    let drained = drainable_locked(&mut cache);
    drop(cache);
    submit_drained(inner, drained);
    Some(processor)
}

pub(super) fn buildable(inner: &FanOutInner, destination: &OtelDestination) -> bool {
    let mut cache = inner.cache.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.closed {
        return false;
    }
    let built = cached_or_built(inner, &mut cache, destination, false).is_some();
    let drained = drainable_locked(&mut cache);
    drop(cache);
    submit_drained(inner, drained);
    built
}

pub(super) fn release(inner: &FanOutInner, processor: &Arc<CachedProcessor>) {
    let mut cache = inner.cache.lock().unwrap_or_else(PoisonError::into_inner);
    let remaining = cache
        .exporting
        .get(&processor.id)
        .copied()
        .unwrap_or(1)
        .saturating_sub(1);
    if remaining > 0 {
        cache.exporting.insert(processor.id, remaining);
    } else {
        cache.exporting.remove(&processor.id);
        if cache.exporting.is_empty() {
            inner.idle.notify_all();
        }
    }
    let drained = drainable_locked(&mut cache);
    drop(cache);
    submit_drained(inner, drained);
}

fn cached_or_built(
    inner: &FanOutInner,
    cache: &mut CacheState,
    destination: &OtelDestination,
    anchored: bool,
) -> Option<Arc<CachedProcessor>> {
    let key = destination.cache_key();
    if let Some(cached) = cache.processors.shift_remove(&key) {
        cache.processors.insert(key, Arc::clone(&cached));
        retire_overflow(inner, cache);
        return Some(cached);
    }
    if !anchored && inner.drain.saturated() {
        tracing::debug!(
            endpoint = %destination.as_ref().endpoint,
            "otel: destination drain saturated, not building"
        );
        return None;
    }
    build_locked(inner, cache, destination, key)
}

fn build_locked(
    inner: &FanOutInner,
    cache: &mut CacheState,
    destination: &OtelDestination,
    key: CacheKey,
) -> Option<Arc<CachedProcessor>> {
    let resource = match inner.resource.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    let built = inner.build.build(destination, &resource)?;
    let id = cache.next_id;
    cache.next_id = cache.next_id.saturating_add(1);
    let cached = Arc::new(CachedProcessor {
        id,
        processor: built,
    });
    cache.processors.insert(key, Arc::clone(&cached));
    retire_overflow(inner, cache);
    Some(cached)
}

fn retire_overflow(inner: &FanOutInner, cache: &mut CacheState) {
    if cache.processors.len() <= MAX_CACHED_PROCESSORS || inner.drain.saturated() {
        return;
    }
    if let Some((_evicted_key, evicted)) = cache.processors.shift_remove_index(0) {
        cache.retired.insert(evicted.id, evicted);
    }
}

fn drainable_locked(cache: &mut CacheState) -> Vec<Arc<CachedProcessor>> {
    let retired = mem::take(&mut cache.retired);
    let mut drained = Vec::new();
    let mut kept = IndexMap::new();
    for (id, processor) in retired {
        if cache.exporting.get(&id).copied().unwrap_or(0) == 0 {
            drained.push(processor);
        } else {
            kept.insert(id, processor);
        }
    }
    cache.retired = kept;
    drained
}

fn submit_drained(inner: &FanOutInner, drained: Vec<Arc<CachedProcessor>>) {
    for retired in drained {
        inner.drain.submit(Arc::clone(&retired.processor));
    }
}
