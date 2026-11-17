use std::error::Error;
use std::future::Future;
use std::time::Duration;

use rust_decimal::Decimal;

use crate::model_budget_values::spend_reads::CachedSpend;

pub trait SessionBudgetCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Script: Send + Sync + 'static;

    // dual_cache.redis_cache is not None
    fn redis_configured(&self) -> bool;

    // redis_cache.async_register_script, called once at construction while Redis is configured
    fn register_increment_script(&self, script: &str) -> Option<Self::Script>;

    // redis_cache.async_get_cache; Err is the exception that sends the read to the in-memory cache
    fn redis_spend(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<CachedSpend>, Self::Error>> + Send;

    // the Lua script's new total as a number; Err sends the increment to the in-memory cache
    fn run_increment_script(
        &self,
        script: &Self::Script,
        key: &str,
        amount: Decimal,
        ttl: Duration,
    ) -> impl Future<Output = Result<Decimal, Self::Error>> + Send;

    // async_get_cache(local_only=True)
    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send;

    // async_set_cache(value, ttl, local_only=True)
    fn memory_set_spend(
        &self,
        key: &str,
        value: CachedSpend,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send;
}
