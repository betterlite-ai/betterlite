#[path = "local_ttl_cache/tests.rs"]
mod tests;

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_CAPACITY: usize = 200;
const DEFAULT_TTL: f64 = 600.0;
const DEFAULT_ITEM_KIB: usize = 1024;
const MAX_KEY_BYTES: usize = 1024;
const MAX_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error("cache capacity cannot be negative")]
    NegativeCapacity,
    #[error("cache value could not be serialized: {0}")]
    ValueSerialization(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CacheValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

pub struct LocalTtlCache {
    capacity: usize,
    default_ttl: f64,
    max_size_per_item: usize,
    values: BTreeMap<String, CacheValue>,
    expiries: BTreeMap<String, f64>,
    heap: BinaryHeap<Reverse<(OrderedFloat, String)>>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OrderedFloat(f64);

impl Eq for OrderedFloat {}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl LocalTtlCache {
    pub fn with_clock(
        capacity: Option<usize>,
        default_ttl: Option<f64>,
        max_size_per_item: Option<usize>,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Result<Self, CacheError> {
        let cache = Self {
            capacity: capacity.unwrap_or(DEFAULT_CAPACITY),
            default_ttl: default_ttl.unwrap_or(DEFAULT_TTL),
            max_size_per_item: max_size_per_item.unwrap_or(DEFAULT_ITEM_KIB),
            values: BTreeMap::new(),
            expiries: BTreeMap::new(),
            heap: BinaryHeap::new(),
            clock,
        };
        Ok(cache)
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        let capacity = self.capacity;
        capacity
    }

    #[must_use]
    pub const fn default_ttl(&self) -> f64 {
        let ttl = self.default_ttl;
        ttl
    }

    #[must_use]
    pub const fn max_size_per_item(&self) -> usize {
        let item_limit = self.max_size_per_item;
        item_limit
    }

    #[must_use]
    pub fn expiry(&self, key: &str) -> Option<f64> {
        let expiry = self.expiries.get(key).copied();
        expiry
    }

    pub fn set(
        &mut self,
        key: &str,
        value: CacheValue,
        ttl: Option<f64>,
    ) -> Result<(), CacheError> {
        self.prune(self.now());
        if self.capacity == 0 || key.len() > MAX_KEY_BYTES {
            return Ok(());
        }
        while self.values.len() >= self.capacity {
            let Some(Reverse((expiry, oldest))) = self.heap.pop() else {
                break;
            };
            if self.expiries.get(&oldest).copied() == Some(expiry.0) {
                self.values.remove(&oldest);
                self.expiries.remove(&oldest);
            }
        }
        if !self.fits(&value)? {
            return Ok(());
        }
        let now = self.now();
        let expiry = self.expiries.get(key).copied();
        self.values.insert(key.to_owned(), value);
        if expiry.is_none_or(|current| current < now) {
            let next = now + ttl.unwrap_or(self.default_ttl);
            self.expiries.insert(key.to_owned(), next);
            self.heap
                .push(Reverse((OrderedFloat(next), key.to_owned())));
        }
        Ok(())
    }

    #[must_use]
    pub fn get(&mut self, key: &str) -> Option<CacheValue> {
        let now = self.now();
        if self.expiries.get(key).is_some_and(|expiry| now > *expiry) {
            self.values.remove(key);
            self.expiries.remove(key);
            return None;
        }
        self.values.get(key).cloned()
    }

    pub fn delete(&mut self, key: &str) {
        self.values.remove(key);
        self.expiries.remove(key);
    }

    fn now(&self) -> f64 {
        let now = (self.clock)();
        now
    }

    fn prune(&mut self, now: f64) {
        while let Some(Reverse((expiry, key))) = self.heap.peek().cloned() {
            let current = self.expiries.get(&key).copied();
            if current != Some(expiry.0) {
                self.heap.pop();
            } else if expiry.0 <= now {
                self.heap.pop();
                self.values.remove(&key);
                self.expiries.remove(&key);
            } else {
                break;
            }
        }
    }

    fn fits(&self, value: &CacheValue) -> Result<bool, CacheError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| CacheError::ValueSerialization(error.to_string()))?;
        let limit = self
            .max_size_per_item
            .checked_mul(1024)
            .unwrap_or(MAX_VALUE_BYTES);
        Ok(bytes.len() <= limit && bytes.len() <= MAX_VALUE_BYTES)
    }
}

#[must_use]
pub fn decode_cached_json(bytes: &[u8]) -> CacheValue {
    let decoded = serde_json::from_slice(bytes).unwrap_or_else(|_| {
        String::from_utf8(bytes.to_vec()).map_or_else(
            |_| {
                CacheValue::Array(
                    bytes
                        .iter()
                        .map(|byte| CacheValue::Integer(i64::from(*byte)))
                        .collect(),
                )
            },
            CacheValue::Text,
        )
    });
    decoded
}
