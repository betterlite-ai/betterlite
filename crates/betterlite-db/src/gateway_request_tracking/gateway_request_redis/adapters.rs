use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::list_operations::RedisListOperations;
use betterlite_redis::list_operations::results::LpopResult;

use crate::queues::pod_lock::LockCache;
use crate::queues::pod_lock::PodLockManager;

use super::GATEWAY_REQUESTS_BUFFER_KEY;
use super::GatewayRequestRedisError;
use super::GatewayRequestsLease;
use super::GatewayRequestsList;
use super::LeaseFuture;
use super::ListFuture;

pub struct RedisGatewayRequestsList {
    pub lists: Arc<RedisListOperations>,
}

impl GatewayRequestsList for RedisGatewayRequestsList {
    // A live list needs a running Redis, which the db tests fake at the GatewayRequestsList seam.
    #[cfg_attr(test, mutants::skip)]
    fn append<'call>(&'call self, entry: &'call str) -> ListFuture<'call, ()> {
        Box::pin(async move {
            let values = [RedisScalar::Text(entry.to_owned())];
            let _pushed = self
                .lists
                .async_rpush(GATEWAY_REQUESTS_BUFFER_KEY, &values)
                .await
                .map_err(|source| GatewayRequestRedisError::List {
                    operation: "async_rpush",
                    source,
                })?;
            Ok(())
        })
    }

    // A live list needs a running Redis, which the db tests fake at the GatewayRequestsList seam.
    #[cfg_attr(test, mutants::skip)]
    fn take(&self, count: usize) -> ListFuture<'_, Vec<String>> {
        Box::pin(async move {
            let requested = i64::try_from(count).unwrap_or(i64::MAX);
            let popped = self
                .lists
                .async_lpop(GATEWAY_REQUESTS_BUFFER_KEY, Some(requested))
                .await
                .map_err(|source| GatewayRequestRedisError::List {
                    operation: "async_lpop",
                    source,
                })?;
            match popped {
                LpopResult::Missing => Ok(Vec::new()),
                LpopResult::Item(text) => Ok(vec![text]),
                LpopResult::Items(texts) => Ok(texts),
                LpopResult::ItemBytes(bytes) => Ok(vec![
                    String::from_utf8(bytes).map_err(GatewayRequestRedisError::Entry)?,
                ]),
                LpopResult::ItemsBytes(values) => values
                    .into_iter()
                    .map(|bytes| String::from_utf8(bytes).map_err(GatewayRequestRedisError::Entry))
                    .collect(),
            }
        })
    }
}

pub struct PodLockLease<Cache: LockCache> {
    pub manager: Arc<PodLockManager<Cache>>,
}

impl<Cache: LockCache> GatewayRequestsLease for PodLockLease<Cache> {
    // A live lease needs a running Redis, which the db tests fake at the GatewayRequestsLease seam.
    #[cfg_attr(test, mutants::skip)]
    fn acquire<'call>(&'call self, job: &'call str) -> LeaseFuture<'call> {
        Box::pin(async move { self.manager.acquire_lock(job, None, true).await })
    }
}
