pub mod codec;
mod operations;

mod tests;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_KEY_BYTES: usize = 1024;
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RouteKey(String);

impl RouteKey {
    pub fn parse(raw: &str) -> Result<Self, StoreError> {
        if raw.len() > MAX_KEY_BYTES {
            return Err(StoreError::KeyTooLong {
                bytes: raw.len(),
                limit: MAX_KEY_BYTES,
            });
        }
        Ok(Self(raw.to_owned()))
    }
}

impl AsRef<str> for RouteKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteValue {
    Counter(i64),
    Spend(Decimal),
    Pin(PinValue),
    SchedulerTuple(SchedulerTuple),
    Members(BTreeSet<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinValue {
    pub model_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerTuple {
    pub priority: u8,
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRead {
    Missing,
    Value(RouteValue),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteIncrement {
    Counter(i64),
    Spend(Decimal),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompareOutcome {
    pub swapped: bool,
    pub current: StoreRead,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoreSettings {
    pub max_size_in_memory: Option<i64>,
    pub default_ttl_seconds: Option<f64>,
    pub max_size_per_item: Option<i64>,
    pub loaded_max_size_per_item_kib: i64,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("a route key of {bytes} bytes is past the {limit}-byte limit")]
    KeyTooLong { bytes: usize, limit: usize },
    #[error("a route value of {bytes} bytes is past the {limit}-byte limit")]
    ValueTooLarge { bytes: usize, limit: usize },
    #[error("a stored route value could not be decoded")]
    InvalidValue {
        #[source]
        source: serde_json::Error,
    },
    #[error("a route value could not be encoded")]
    Encoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cached value is not a route value")]
    NotARouteValue,
    #[error("the value under {key} is not a member set")]
    NotMembers { key: String },
    #[error("the value under {key} is not an incrementable counter or spend")]
    NotIncrementable { key: String },
    #[error("the counter under {key} overflowed")]
    CounterOverflow { key: String },
    #[error("the spend under {key} overflowed")]
    SpendOverflow { key: String },
    #[error("the local cache failed")]
    Cache {
        #[from]
        source: CacheError,
    },
}

pub struct RoutingValueStore {
    cache: Mutex<LocalTtlCache>,
}
