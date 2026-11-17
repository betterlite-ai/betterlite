use std::num::TryFromIntError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use thiserror::Error;

use crate::straiker::delivery::{BLOCKED_TURN_ITEM_KIB, BLOCKED_TURN_TTL_SECONDS};
use crate::straiker::v3::BLOCKED_TURN_MEMORY;

#[derive(Debug, Error)]
pub enum BlockedTurnError {
    #[error("the blocked-turn memory size does not fit the cache")]
    Capacity(#[source] TryFromIntError),
    #[error(transparent)]
    Cache(#[from] CacheError),
}

// LiteLLM's InMemoryCache of blocked conversations, keyed by "<scope>\0<prefix>"; the process that saw the block answers the replay itself.
pub struct BlockedTurns {
    cache: Mutex<LocalTtlCache>,
}

impl BlockedTurns {
    pub fn new(clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Result<Self, BlockedTurnError> {
        let capacity = i64::try_from(BLOCKED_TURN_MEMORY).map_err(BlockedTurnError::Capacity)?;
        let cache = LocalTtlCache::with_clock(
            Some(capacity),
            Some(BLOCKED_TURN_TTL_SECONDS),
            None,
            BLOCKED_TURN_ITEM_KIB,
            clock,
        )?;
        Ok(Self {
            cache: Mutex::new(cache),
        })
    }

    pub fn remember(&self, scope: &str, prefix: &str, message: &str) -> Result<(), CacheError> {
        let key = format!("{scope}\0{prefix}");
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set(&key, CacheValue::Text(message.to_owned()), None)
    }

    pub fn replayed(
        &self,
        scope: &str,
        prefixes: &[String],
    ) -> Result<Option<(String, String)>, CacheError> {
        for prefix in prefixes {
            let key = format!("{scope}\0{prefix}");
            let cached = self
                .cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&key)?;
            let Some(value) = cached else {
                continue;
            };
            if let Some(message) = replay_message(&value) {
                return Ok(Some((prefix.clone(), message)));
            }
        }
        Ok(None)
    }
}

// The cache decodes the stored str as JSON, so a literal null is no block and any other decoded value blocks with its JSON text.
fn replay_message(value: &CacheValue) -> Option<String> {
    match value {
        CacheValue::Null => None,
        CacheValue::Text(message) => Some(message.clone()),
        CacheValue::Credential(_)
        | CacheValue::Bytes(_)
        | CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => serde_json::to_string(value).ok(),
    }
}

#[must_use]
pub fn system_clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
    })
}
