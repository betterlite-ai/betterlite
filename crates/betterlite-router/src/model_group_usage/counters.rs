use std::convert::Infallible;
use std::error::Error;

use async_trait::async_trait;
use betterlite_cache::batch_reads::{BatchReadOptions, BatchReads};
use betterlite_models::decisions::Json;

// Router.cache.async_batch_get_cache as the usage reads use it: one ordered value per key, or no read at all.
#[async_trait]
pub trait UsageCounterCache: Send + Sync {
    type Error: Error + Send + 'static;

    async fn batch_get(&self, keys: &[String]) -> Result<Option<Vec<Option<Json>>>, Self::Error>;
}

// A router cache behind the counter seam, read with the keyword defaults both Router calls pass: no local_only, Redis throttled.
pub struct BatchCounters<'cache, C> {
    pub cache: &'cache C,
}

#[async_trait]
impl<C> UsageCounterCache for BatchCounters<'_, C>
where
    C: BatchReads + Send + Sync,
{
    type Error = Infallible;

    async fn batch_get(&self, keys: &[String]) -> Result<Option<Vec<Option<Json>>>, Infallible> {
        let options = BatchReadOptions {
            local_only: false,
            throttle_redis: true,
        };
        Ok(BatchReads::async_batch_get_cache(self.cache, keys, options).await)
    }
}
