mod tests;

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use rust_decimal::Decimal;

use crate::model_budget_values::amount_text::amount_from_text;
use crate::model_budget_values::spend_reads::CachedSpend;

use super::ModelBudgetLimiter;

pub trait ModelBudgetCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn redis_configured(&self) -> bool;

    // a Redis failure reads as a miss, as RedisCache.async_get_cache returns None on error
    fn redis_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send;

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send;

    // adds one with the TTL refreshed on every write, as async_increment_cache(refresh_ttl=True) does
    fn increment_counter(
        &self,
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = Option<Decimal>> + Send;

    // B3016's increment_spend_for_key: queue the cost and update the in-memory window counter
    fn increment_spend(
        &self,
        duration: Duration,
        spend_key: &str,
        start_time_key: &str,
        cost: Decimal,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    // B3016's flush_increments: write queued increments to Redis; a failed batch is kept and logged there
    fn flush_increments(&self) -> impl Future<Output = ()> + Send;
}

pub(crate) async fn cached_spend<C>(limiter: &ModelBudgetLimiter<C>, key: &str) -> Decimal
where
    C: ModelBudgetCache,
{
    let cached = if limiter.cache.redis_configured() {
        limiter.cache.redis_spend(key).await
    } else {
        limiter.cache.memory_spend(key).await
    };
    match cached {
        Some(CachedSpend::Number(amount)) => amount,
        Some(CachedSpend::Text(text)) => amount_from_text(text.trim()).unwrap_or(Decimal::ZERO),
        Some(CachedSpend::Unreadable) | None => Decimal::ZERO,
    }
}
