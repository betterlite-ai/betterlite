use super::{CacheError, CacheLimits, CacheValue, DEFAULT_CAPACITY, DEFAULT_TTL, LocalTtlCache};
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_redact::secret::Redactor;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug)]
pub enum Ttl {
    Absent,
    Explicit(Option<f64>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum EventKind {
    Verbose,
    Error,
}

#[derive(Debug)]
pub struct CacheEvent {
    pub kind: EventKind,
    pub message: String,
}

pub struct Diagnostics {
    pub redactor: Arc<Redactor>,
    pub emit: Arc<dyn Fn(CacheEvent) + Send + Sync>,
}

pub struct DualLocalCache {
    pub memory: Arc<Mutex<LocalTtlCache>>,
    pub default_in_memory_ttl: Option<f64>,
    pub diagnostics: Diagnostics,
}

impl DualLocalCache {
    pub fn new(
        supplied: Option<Arc<Mutex<LocalTtlCache>>>,
        constructor_ttl: Option<f64>,
        settings: &Settings,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
        diagnostics: Diagnostics,
    ) -> Self {
        let memory = supplied.unwrap_or_else(|| {
            Arc::new(Mutex::new(LocalTtlCache {
                limits: CacheLimits {
                    capacity: DEFAULT_CAPACITY,
                    default_ttl: DEFAULT_TTL,
                    max_size_per_item: 1024,
                },
                values: BTreeMap::new(),
                expiries: BTreeMap::new(),
                heap: BinaryHeap::new(),
                clock,
            }))
        });
        Self {
            memory,
            default_in_memory_ttl: constructor_ttl
                .filter(|ttl| *ttl != 0.0_f64)
                .or(settings.global_ttl),
            diagnostics,
        }
    }

    pub fn set(&self, key: &str, value: CacheValue, ttl: Ttl) {
        let resolved = match ttl {
            Ttl::Absent => self.default_in_memory_ttl,
            Ttl::Explicit(explicit) => explicit,
        };
        let result = self
            .memory
            .lock()
            .map_err(|_poisoned| CacheError::Unavailable)
            .and_then(|mut memory| memory.set(key, value, resolved));
        if let Err(error) = result {
            self.event(EventKind::Verbose, "set cache", &error.to_string());
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<CacheValue> {
        let result = self
            .memory
            .lock()
            .map_err(|_poisoned| CacheError::Unavailable)
            .and_then(|mut memory| memory.get(key));
        match result {
            Ok(value) => {
                let hit = value.filter(|cached| *cached != CacheValue::Null);
                self.event(
                    EventKind::Verbose,
                    "get cache: cache result",
                    "cached value",
                );
                hit
            }
            Err(error) => {
                self.event(
                    EventKind::Error,
                    "LiteLLM Cache: exception in get_cache",
                    &error.to_string(),
                );
                None
            }
        }
    }

    fn event(&self, kind: EventKind, message: &str, detail: &str) {
        let redacted = self
            .diagnostics
            .redactor
            .redact_structured_value(Some("cache_secret"), detail);
        (self.diagnostics.emit)(CacheEvent {
            kind,
            message: format!("{message}: {redacted}"),
        });
    }
}
