pub mod batch;
pub mod cache;
pub mod error;
pub mod request;

mod recheck;
mod tests;

use betterlite_db::spend_rows::key::END_USER_COUNTER_PREFIX;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use rust_decimal::Decimal;

use self::batch::{NoSpendBatch, SpendBatch};
use self::cache::{SPEND_DB_FLOOR_MARKER_PREFIX, SpendReadCache, cache_estimate};
use self::error::BudgetUnverifiable;
use self::recheck::{Recheck, recheck_spend};
use self::request::{SpendReadRequest, SpendWindow};
use crate::auth_invalidation::SPEND_DB_FLOOR_TTL_SECONDS;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::SpendCounterRows;

pub struct SpendReads<'reads, Rows, Cache, Batch>
where
    Rows: SpendCounterRows,
    Cache: SpendReadCache,
    Batch: SpendBatch,
{
    pub reseed: &'reads SpendCounterReseed<Rows>,
    pub cache: &'reads Cache,
    pub batch: Option<&'reads Batch>,
    pub fail_closed_budget_enforcement: bool,
}

impl<Rows, Cache, Batch> SpendReads<'_, Rows, Cache, Batch>
where
    Rows: SpendCounterRows + DbSpendCounterRows,
    Cache: SpendReadCache,
    Batch: SpendBatch,
{
    pub async fn get_current_spend(
        &self,
        request: &SpendReadRequest<'_>,
    ) -> Result<Decimal, BudgetUnverifiable> {
        let counter_key = request.counter_key;
        let (current, mut verified) = self
            .read_spend_counter_estimate(counter_key, request.fallback_spend)
            .await;
        if request.fallback_authoritative {
            verified = true;
        }
        let Some(max_budget) = request.max_budget else {
            return Ok(current);
        };
        if current >= max_budget {
            return Ok(current);
        }
        match recheck_spend(self, request, current).await {
            Recheck::Raised(amount) => return Ok(amount),
            Recheck::Verified => verified = true,
            Recheck::Unchanged => {}
        }
        if verified || !self.fail_closed_budget_enforcement {
            return Ok(current);
        }
        tracing::warn!(
            counter_key,
            "fail_closed_budget_enforcement: rejecting a request whose spend could not be verified against Redis or the database"
        );
        Err(BudgetUnverifiable {
            counter_key: counter_key.to_owned(),
        })
    }

    pub async fn reseed_spend_counter_from_db(&self, counter_key: &str) -> bool {
        let Some(db_spend) = self.reseed.reader().from_db(counter_key).await else {
            return false;
        };
        self.repair_stale_spend_counter(counter_key, db_spend).await;
        true
    }

    pub async fn repair_stale_spend_counter(&self, counter_key: &str, db_spend: Decimal) {
        let cached = self.memory_value(counter_key).await;
        // no await between the compare and this write, so the update is atomic within this pod
        if cached.is_none_or(|amount| amount < db_spend)
            && let Err(error) = self.cache.memory_set_ttl(counter_key, db_spend, None)
        {
            tracing::debug!(
                counter_key,
                %error,
                "the stale spend counter's in-memory copy could not be repaired"
            );
        }
        if self.cache.redis_configured() {
            self.repair_redis(counter_key, db_spend).await;
        }
    }

    // the batch forgets the key first so a batched increment cannot overwrite the repair
    async fn repair_redis(&self, counter_key: &str, db_spend: Decimal) {
        if let Some(batch) = self.batch {
            batch.forget(counter_key);
        }
        let repaired = match self.cache.redis_set_max(counter_key, db_spend).await {
            Ok(repaired) => repaired,
            Err(error) => {
                tracing::debug!(
                    counter_key,
                    %error,
                    "Unable to repair stale spend counter in Redis"
                );
                return;
            }
        };
        if let Some(value) = repaired
            && let Some(batch) = self.batch
        {
            batch.record(counter_key, value);
        }
    }

    pub async fn read_spend_counter_cache_value(
        &self,
        counter_key: &str,
    ) -> (Option<Decimal>, bool) {
        if let Some(batch) = self.batch {
            match batch.read(counter_key).await {
                Ok(Some(CounterValue::Amount(amount))) => return (Some(amount), true),
                Ok(Some(CounterValue::Missing)) => return (None, true),
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(
                        counter_key,
                        %error,
                        "the spend counter batch read failed; reading Redis directly"
                    );
                }
            }
        }
        if self.cache.redis_configured()
            && let Some(answer) = self.redis_spend(counter_key).await
        {
            return answer;
        }
        (self.memory_value(counter_key).await, false)
    }

    // None means the Redis tier could not answer, and the in-memory tier is read next
    async fn redis_spend(&self, counter_key: &str) -> Option<(Option<Decimal>, bool)> {
        let read = self.cache.redis_get(counter_key).await;
        if let Err(error) = &read {
            tracing::debug!(
                counter_key,
                %error,
                "the Redis spend counter read failed; falling back to memory"
            );
            return None;
        }
        let Some(stored) = read.ok().flatten() else {
            return Some((None, true));
        };
        cache_estimate(&stored, counter_key, "Redis").map(|amount| (Some(amount), true))
    }

    pub async fn read_spend_counter_estimate(
        &self,
        counter_key: &str,
        fallback_spend: Decimal,
    ) -> (Decimal, bool) {
        let (cached, authoritative) = self.read_spend_counter_cache_value(counter_key).await;
        if let Some(current) = cached {
            return (current, authoritative);
        }
        match self.coalesced_spend(counter_key).await {
            Some(db_spend) => (db_spend, true),
            None => (fallback_spend, false),
        }
    }

    pub async fn authoritative_floor_spend(
        &self,
        counter_key: &str,
        window: Option<&SpendWindow<'_>>,
    ) -> Option<Decimal> {
        let marker_key = format!("{SPEND_DB_FLOOR_MARKER_PREFIX}{counter_key}");
        if let Some(cached) = self.memory_value(&marker_key).await {
            return Some(cached);
        }
        let db_spend = self.floor_spend_from_db(counter_key, window).await?;
        // a spend reset that committed during the read above wrote the post-reset floor here
        if let Some(fresher) = self.memory_value(&marker_key).await {
            return Some(fresher);
        }
        if let Err(error) =
            self.cache
                .memory_set_ttl(&marker_key, db_spend, Some(SPEND_DB_FLOOR_TTL_SECONDS))
        {
            tracing::debug!(marker_key, %error, "the spend floor marker could not be cached");
        }
        Some(db_spend)
    }

    pub async fn floor_spend_from_db(
        &self,
        counter_key: &str,
        window: Option<&SpendWindow<'_>>,
    ) -> Option<Decimal> {
        let reader = self.reseed.reader();
        if counter_key.starts_with(END_USER_COUNTER_PREFIX) {
            return reader.end_user_from_db(counter_key).await;
        }
        if let Some(entity_spend) = reader.from_db(counter_key).await {
            return Some(entity_spend);
        }
        let Some(window) = window else {
            return None;
        };
        match self
            .reseed
            .window_from_db(
                window.entity_type,
                window.entity_id,
                window.window_duration,
                window.window_start,
            )
            .await
        {
            Ok(spend) => spend,
            Err(error) => {
                tracing::error!(
                    counter_key,
                    %error,
                    "the window spend floor read failed; the caller's fallback applies"
                );
                None
            }
        }
    }

    async fn coalesced_spend(&self, counter_key: &str) -> Option<Decimal> {
        let outcome = match self.batch {
            Some(batch) => {
                self.reseed
                    .coalesced(self.cache, batch, counter_key, false)
                    .await
            }
            None => {
                let absent = NoSpendBatch;
                self.reseed
                    .coalesced(self.cache, &absent, counter_key, false)
                    .await
            }
        };
        outcome.unwrap_or_else(|error| {
            tracing::error!(
                counter_key,
                %error,
                "the spend counter reseed failed; using the caller's fallback"
            );
            None
        })
    }

    async fn memory_value(&self, key: &str) -> Option<Decimal> {
        let read = self.cache.memory_get(key).await;
        if let Err(error) = &read {
            tracing::debug!(key, %error, "the in-memory spend counter read failed");
        }
        let value = read.ok().flatten()?;
        cache_estimate(&value, key, "in-memory")
    }
}
