mod tests;

use std::sync::Arc;

use super::RefreshCoordinator;
use super::distributed_lock::{RedisCommands, RedisDistributedLock};
use super::refresh_coordinator::{RedisRefreshCoordinator, RedisTunables};

// The composition root's Redis answer, asked once per store build, the way LiteLLM reads user_api_key_cache.redis_cache lazily since Redis may come up after start.
#[must_use]
pub fn runtime_refresh_coordinator<Client, Output, Error>(
    client: Option<Client>,
    namespace: Option<&str>,
    tunables: &RedisTunables,
) -> Option<Arc<dyn RefreshCoordinator<Output = Output, Error = Error>>>
where
    Client: RedisCommands + 'static,
    Output: Clone + Send + 'static,
    Error: Clone + Send + 'static,
{
    let mut coordinator = RedisRefreshCoordinator::new(RedisDistributedLock {
        client: client?,
        namespace: namespace.map(str::to_owned),
    });
    coordinator.tunables = tunables.clone();
    Some(Arc::new(coordinator))
}
