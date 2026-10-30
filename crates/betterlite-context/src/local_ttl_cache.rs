#[path = "local_ttl_cache/tests.rs"]
mod tests;

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;
use std::num::TryFromIntError;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_CAPACITY: usize = 200;
const DEFAULT_TTL: f64 = 600.0;
pub mod credential;
pub mod dual;
use credential::CredentialHandle;
const MAX_KEY_BYTES: usize = 1024;
const MAX_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error("cache capacity cannot be negative")]
    NegativeCapacity {
        #[source]
        source: TryFromIntError,
    },
    #[error("cache expiration heap is empty")]
    MissingExpiry,
    #[error("cache lock is poisoned")]
    Unavailable,
    #[error("cache clock is not finite")]
    InvalidClock,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CacheValue {
    #[serde(skip)]
    Credential(CredentialHandle),
    #[serde(skip)]
    Bytes(Vec<u8>),
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug)]
pub struct CacheLimits {
    pub capacity: usize,
    pub default_ttl: f64,
    pub max_size_per_item: i64,
}

pub struct LocalTtlCache {
    limits: CacheLimits,
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
        capacity: Option<i64>,
        default_ttl: Option<f64>,
        max_size_per_item: Option<i64>,
        loaded_item_kib: i64,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Result<Self, CacheError> {
        let resolved_capacity = capacity
            .map_or(Ok(DEFAULT_CAPACITY), usize::try_from)
            .map_err(|source| CacheError::NegativeCapacity { source })?;
        let cache = Self {
            limits: CacheLimits {
                capacity: resolved_capacity,
                default_ttl: default_ttl
                    .filter(|ttl| *ttl != 0.0_f64)
                    .unwrap_or(DEFAULT_TTL),
                max_size_per_item: max_size_per_item
                    .filter(|size| *size != 0)
                    .unwrap_or(loaded_item_kib),
            },
            values: BTreeMap::new(),
            expiries: BTreeMap::new(),
            heap: BinaryHeap::new(),
            clock,
        };
        Ok(cache)
    }

    pub fn set(
        &mut self,
        key: &str,
        value: CacheValue,
        ttl: Option<f64>,
    ) -> Result<(), CacheError> {
        let initial_time = self.read_clock()?;
        self.prune(initial_time);
        if self.limits.capacity == 0 || key.len() > MAX_KEY_BYTES {
            return Ok(());
        }
        while self.values.len() >= self.limits.capacity {
            let Some(Reverse((expiry, oldest))) = self.heap.pop() else {
                return Err(CacheError::MissingExpiry);
            };
            if self.expiries.get(&oldest).copied() == Some(expiry.0) {
                self.values.remove(&oldest);
                self.expiries.remove(&oldest);
            }
        }
        if !self.fits(&value) {
            return Ok(());
        }
        let now = self.read_clock()?;
        let expiry = self.expiries.get(key).copied();
        self.values.insert(key.to_owned(), value);
        if expiry.is_none_or(|current| current < now) {
            let next = now.mul_add(1.0_f64, ttl.unwrap_or(self.limits.default_ttl));
            self.expiries.insert(key.to_owned(), next);
            self.heap
                .push(Reverse((OrderedFloat(next), key.to_owned())));
        }
        Ok(())
    }

    pub fn get(&mut self, key: &str) -> Result<Option<CacheValue>, CacheError> {
        let now = self.read_clock()?;
        if self.expiries.get(key).is_some_and(|expiry| now > *expiry) {
            self.values.remove(key);
            self.expiries.remove(key);
            return Ok(None);
        }
        Ok(match self.values.get(key) {
            Some(CacheValue::Text(text)) => Some(decode_cached_json(text.as_bytes())),
            Some(CacheValue::Bytes(bytes)) => Some(decode_cached_json(bytes)),
            value => value.cloned(),
        })
    }

    pub fn delete(&mut self, key: &str) {
        self.values.remove(key);
        self.expiries.remove(key);
    }

    fn read_clock(&self) -> Result<f64, CacheError> {
        let now = (self.clock)();
        if now.is_finite() {
            Ok(now)
        } else {
            Err(CacheError::InvalidClock)
        }
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

    fn fits(&self, value: &CacheValue) -> bool {
        let charge = match value {
            CacheValue::Credential(handle) => handle.size_charge(),
            CacheValue::Bytes(bytes) => bytes.len(),
            CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Integer(_)
            | CacheValue::Float(_)
            | CacheValue::Text(_)
            | CacheValue::Array(_)
            | CacheValue::Object(_) => match serde_json::to_vec(value) {
                Ok(bytes) => bytes.len(),
                Err(_) => return false,
            },
        };
        let limit = usize::try_from(self.limits.max_size_per_item)
            .unwrap_or(0)
            .saturating_mul(1024)
            .min(MAX_VALUE_BYTES);
        charge <= limit
    }
}

#[must_use]
pub fn decode_cached_json(bytes: &[u8]) -> CacheValue {
    if bytes.len() > MAX_VALUE_BYTES {
        return CacheValue::Null;
    }
    let parsed = serde_json::from_slice(bytes);
    parsed.unwrap_or_else(|_| {
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
    })
}

impl fmt::Debug for LocalTtlCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTtlCache")
            .field("entries", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl AsRef<CacheLimits> for LocalTtlCache {
    fn as_ref(&self) -> &CacheLimits {
        &self.limits
    }
}
