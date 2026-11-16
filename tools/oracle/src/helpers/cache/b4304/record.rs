use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::lookup::CacheLookup;
use betterlite_context::local_ttl_cache::CacheValue;
use serde_json::Value;

// Which tier a write landed on: LiteLLM's dynamic_cache_object when one was given, its own backend otherwise.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Sink {
    Backend,
    Dual,
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) writes: Mutex<Vec<Value>>,
}

impl Recorder {
    pub(crate) fn record(&self, sink: Sink, key: &str, envelope: &CacheValue) {
        let write = serde_json::json!({
            "sink": sink_name(sink),
            "kind": stored_kind(envelope),
            "key": key,
        });
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.push(write);
    }
}

const fn sink_name(sink: Sink) -> &'static str {
    match sink {
        Sink::Backend => "backend",
        Sink::Dual => "dual",
    }
}

// LiteLLM stores a typed response as model_dump_json's string and every other result as the value itself.
fn stored_kind(envelope: &CacheValue) -> &'static str {
    match envelope {
        CacheValue::Object(fields) => match fields.get("response") {
            Some(CacheValue::Text(_)) => "text",
            Some(_) | None => "value",
        },
        _ => "value",
    }
}

pub(crate) struct RecordingBackend {
    pub(crate) recorder: Arc<Recorder>,
}

#[async_trait]
impl EnvelopeBackend for RecordingBackend {
    fn get_cache(&self, _key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Backend, key, &value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        _key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Backend, key, &value);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Backend, key, &value);
        Ok(())
    }
}

pub(crate) struct RecordingMemory {
    pub(crate) recorder: Arc<Recorder>,
    pub(crate) stored: Mutex<BTreeMap<String, CacheValue>>,
}

impl MemoryCacheOps for RecordingMemory {
    fn default_ttl(&self) -> i64 {
        600
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(stored.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.recorder.record(Sink::Dual, key, &value);
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.insert(key.to_owned(), value);
        Ok(())
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.remove(key);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.clear();
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }

    fn increment_cache(&self, _key: &str, value: f64, _ttl: Ttl) -> CacheResult<f64> {
        Ok(value)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Ok(Vec::new())
    }
}
