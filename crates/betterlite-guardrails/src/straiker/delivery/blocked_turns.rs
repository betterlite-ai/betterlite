use std::collections::BTreeMap;
use std::num::TryFromIntError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use thiserror::Error;

use crate::straiker::delivery::{BLOCKED_TURN_ITEM_KIB, BLOCKED_TURN_TTL_SECONDS};
use crate::straiker::v3::BLOCKED_TURN_MEMORY;

const MESSAGE_FIELD: &str = "message";

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
        let value = CacheValue::Object(BTreeMap::from([(
            MESSAGE_FIELD.to_owned(),
            CacheValue::Text(message.to_owned()),
        )]));
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set(&key, value, None)
    }

    pub fn replayed(
        &self,
        scope: &str,
        prefixes: &[String],
    ) -> Result<Option<(String, String)>, CacheError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        for prefix in prefixes {
            let key = format!("{scope}\0{prefix}");
            let Some(CacheValue::Object(fields)) = cache.get(&key)? else {
                continue;
            };
            let Some(CacheValue::Text(message)) = fields.get(MESSAGE_FIELD) else {
                continue;
            };
            return Ok(Some((prefix.clone(), message.clone())));
        }
        Ok(None)
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
