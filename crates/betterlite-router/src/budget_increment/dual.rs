use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use rust_decimal::Decimal;

use super::error::IncrementError;
use super::queue::QueuedIncrement;
use super::window::{WindowCache, WindowRedis};

const SPEND_FIELD: &str = "spend";

pub struct DualWindowCache<R: WindowRedis> {
    pub memory: Arc<Mutex<LocalTtlCache>>,
    pub redis: Option<R>,
}

#[async_trait]
impl<R: WindowRedis> WindowCache for DualWindowCache<R> {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        let cached = self
            .memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)?;
        match cached {
            Some(CacheValue::Float(seconds)) => return Ok(Some(seconds)),
            Some(_) => {
                return Err(IncrementError::InvalidCacheValue {
                    key: key.to_owned(),
                });
            }
            None => {}
        }
        let Some(redis) = &self.redis else {
            return Ok(None);
        };
        let Some(seconds) = redis.read_start_time(key).await? else {
            return Ok(None);
        };
        self.set_memory(key, CacheValue::Float(seconds), None)?;
        Ok(Some(seconds))
    }

    async fn write_start_time(
        &self,
        key: &str,
        start_time: f64,
        ttl: Duration,
    ) -> Result<(), IncrementError> {
        self.set_memory(key, CacheValue::Float(start_time), Some(ttl))?;
        if let Some(redis) = &self.redis {
            redis.write_start_time(key, start_time, ttl).await;
        }
        Ok(())
    }

    async fn reset_spend(
        &self,
        key: &str,
        cost: Decimal,
        ttl: Duration,
    ) -> Result<(), IncrementError> {
        let encoded = CacheValue::Object(BTreeMap::from([(
            SPEND_FIELD.to_owned(),
            CacheValue::Text(cost.to_string()),
        )]));
        self.set_memory(key, encoded, Some(ttl))?;
        if let Some(redis) = &self.redis {
            redis.write_spend(key, cost, ttl).await;
        }
        Ok(())
    }

    async fn increment_spend(
        &self,
        key: &str,
        cost: Decimal,
        ttl: Duration,
    ) -> Result<Decimal, IncrementError> {
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        let current = match memory.get(key)? {
            Some(value) => {
                decode_spend(&value).ok_or_else(|| IncrementError::InvalidCacheValue {
                    key: key.to_owned(),
                })?
            }
            None => Decimal::ZERO,
        };
        let next = current
            .checked_add(cost)
            .ok_or_else(|| IncrementError::SpendOverflow {
                key: key.to_owned(),
            })?;
        memory.delete(key);
        let encoded = CacheValue::Object(BTreeMap::from([(
            SPEND_FIELD.to_owned(),
            CacheValue::Text(next.to_string()),
        )]));
        memory.set(key, encoded, Some(ttl.as_secs_f64()))?;
        drop(memory);
        Ok(next)
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        match &self.redis {
            Some(redis) => redis.write_increments(operations).await,
            None => true,
        }
    }
}

impl<R: WindowRedis> DualWindowCache<R> {
    fn set_memory(
        &self,
        key: &str,
        value: CacheValue,
        ttl: Option<Duration>,
    ) -> Result<(), IncrementError> {
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.delete(key);
        memory.set(key, value, ttl.map(|duration| duration.as_secs_f64()))?;
        drop(memory);
        Ok(())
    }
}

fn decode_spend(value: &CacheValue) -> Option<Decimal> {
    let CacheValue::Object(fields) = value else {
        return None;
    };
    let CacheValue::Text(text) = fields.get(SPEND_FIELD)? else {
        return None;
    };
    Decimal::from_str(text).ok()
}
