use std::error::Error;
use std::future::Future;
use std::time::Duration;

pub trait SessionIterationCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Script: Send + Sync + 'static;

    // dual_cache.redis_cache is not None
    fn redis_configured(&self) -> bool;

    // redis_cache.async_register_script, called once at construction while Redis is configured
    fn register_increment_script(&self, script: &str) -> Option<Self::Script>;

    // the registered Lua script over one session counter; Err is the exception that sends the increment to the local cache
    fn run_increment_script(
        &self,
        script: &Self::Script,
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send;

    // async_get_cache(local_only=True) with int() applied: the stored count or its text, and none once its ttl has passed
    fn memory_count(&self, key: &str) -> impl Future<Output = Option<i64>> + Send;

    // async_set_cache(value=new_value, ttl=self.ttl, local_only=True)
    fn memory_set_count(
        &self,
        key: &str,
        value: i64,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send;
}
