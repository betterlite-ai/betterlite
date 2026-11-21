pub mod cache;
mod floor;

use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::budget_counter::CounterEntityType;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::reseed::{CoalescedError, SpendCounterReseed};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::values::{SpendValueError, cache_amount};

use self::cache::{CounterBatch, CounterCache};
use self::floor::{FloorCheck, recheck_floor};

// bounds the authoritative re-reads of a stale-low counter to one per counter per window
pub const FLOOR_MARKER_TTL_SECONDS: u32 = 5;

pub const BUDGET_UNVERIFIABLE_DETAIL: &str = "Budget enforcement unavailable: current spend could not be verified against Redis or the database, and fail_closed_budget_enforcement is enabled, so the request was rejected to avoid exceeding the configured budget. Retry shortly.";

#[derive(Debug, Error)]
pub enum CurrentSpendError<CacheError> {
    #[error("the spend for {counter_key} could not be verified against Redis or the database")]
    Unverifiable { counter_key: String },
    #[error(transparent)]
    Cache(CacheError),
    #[error(transparent)]
    Value(#[from] SpendValueError),
    #[error(transparent)]
    Batch(#[from] SpendCounterBatchError),
    #[error(transparent)]
    Reseed(#[from] CoalescedError<CacheError>),
}

pub type SpendResult<Value, Cache> =
    Result<Value, CurrentSpendError<<Cache as ReseedCache>::Error>>;

// each descriptor is optional on its own: the window flag reads only the start, the window read needs type, id and start
#[derive(Clone, Copy, Debug)]
pub struct WindowDescriptors<'query> {
    pub entity_type: Option<&'query CounterEntityType>,
    pub entity_id: Option<&'query str>,
    pub duration: Option<&'query str>,
    pub start: Option<Timestamp>,
}

#[derive(Clone, Copy, Debug)]
pub struct SpendQuery<'query> {
    pub counter_key: &'query str,
    pub fallback_spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub window: WindowDescriptors<'query>,
    pub fallback_authoritative: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CounterReading {
    pub value: Option<Decimal>,
    // true when the value came from the batch, Redis or a fresh database read, false for a per-pod copy
    pub authoritative: bool,
}

// Unusable sends the read to the per-pod copy: the Redis read failed or held a value float() rejects
enum RedisAnswer {
    Read(Option<Decimal>),
    Unusable,
}

pub struct CurrentSpend<'deps, Cache: ?Sized, Batch: ?Sized, Rows: SpendCounterRows> {
    pub cache: &'deps Cache,
    pub batch: &'deps Batch,
    pub reseed: &'deps SpendCounterReseed<Rows>,
    pub fail_closed_budget_enforcement: bool,
}

impl<Cache, Batch, Rows> CurrentSpend<'_, Cache, Batch, Rows>
where
    Cache: CounterCache + ?Sized,
    Batch: CounterBatch + ?Sized,
    Rows: SpendCounterRows,
{
    pub async fn get_current_spend(&self, query: &SpendQuery<'_>) -> SpendResult<Decimal, Cache> {
        let (current, estimate_verified) = self
            .read_spend_counter_estimate(query.counter_key, query.fallback_spend)
            .await?;
        let Some(max_budget) = query.max_budget else {
            return Ok(current);
        };
        if current >= max_budget {
            return Ok(current);
        }
        let verified = match recheck_floor(self, query, current).await? {
            FloorCheck::Settled(spend) => return Ok(spend),
            FloorCheck::Verified => true,
            FloorCheck::Skipped => estimate_verified || query.fallback_authoritative,
        };
        if !verified && self.fail_closed_budget_enforcement {
            tracing::warn!(
                counter_key = query.counter_key,
                "fail_closed_budget_enforcement: rejecting request, the spend could not be verified against Redis or the database"
            );
            return Err(CurrentSpendError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            });
        }
        Ok(current)
    }

    // a clean Redis miss is final: the per-pod copy outlives the Redis TTL and holds only this pod's writes
    pub async fn read_spend_counter_cache_value(
        &self,
        counter_key: &str,
    ) -> SpendResult<CounterReading, Cache> {
        if let Some(batched) = self.batch.read(counter_key).await? {
            let value = match batched {
                CounterValue::Amount(amount) => Some(amount),
                CounterValue::Missing => None,
            };
            return Ok(CounterReading {
                value,
                authoritative: true,
            });
        }
        if self.cache.redis_configured()
            && let RedisAnswer::Read(value) = self.redis_amount(counter_key).await
        {
            return Ok(CounterReading {
                value,
                authoritative: true,
            });
        }
        Ok(CounterReading {
            value: self.local_amount(counter_key).await?,
            authoritative: false,
        })
    }

    async fn redis_amount(&self, counter_key: &str) -> RedisAnswer {
        let stored = match self.cache.redis_get(counter_key).await {
            Ok(stored) => stored,
            Err(error) => {
                tracing::debug!(counter_key, %error, "get_current_spend: Redis read failed, falling back to in-memory");
                return RedisAnswer::Unusable;
            }
        };
        match stored.as_ref().map(cache_amount).transpose() {
            Ok(amount) => RedisAnswer::Read(amount),
            Err(error) => {
                tracing::debug!(counter_key, %error, "get_current_spend: the Redis value is not a number, falling back to in-memory");
                RedisAnswer::Unusable
            }
        }
    }

    async fn read_spend_counter_estimate(
        &self,
        counter_key: &str,
        fallback_spend: Decimal,
    ) -> SpendResult<(Decimal, bool), Cache> {
        let reading = self.read_spend_counter_cache_value(counter_key).await?;
        if let Some(value) = reading.value {
            return Ok((value, reading.authoritative));
        }
        // the caller's fallback lags the other pods, so a cold counter reseeds from the database first
        let reseeded = self
            .reseed
            .coalesced(self.cache, self.batch, counter_key, false)
            .await?;
        Ok(reseeded.map_or((fallback_spend, false), |db_spend| (db_spend, true)))
    }

    // Python's float(value) on a per-pod hit has no guard, so an unreadable value raises
    async fn local_amount(&self, key: &str) -> SpendResult<Option<Decimal>, Cache> {
        self.cache
            .memory_get(key)
            .await
            .map_err(CurrentSpendError::Cache)?
            .as_ref()
            .map(cache_amount)
            .transpose()
            .map_err(CurrentSpendError::Value)
    }
}

mod tests;
